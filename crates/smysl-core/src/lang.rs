//! Whether a unit is written in the script of the passage it came from (F-6, `SMY-W436`).
//!
//! `LangPolicy::Source` says each gist is written in the language of the passage it was drawn
//! from. 1.9 shipped the policy, the prompt that states it, and no way to tell whether a model
//! obeyed — and the S0 spike measured both failure directions on one corpus. The local model
//! wrote **78.6% of its Russian gists in Latin script**, answering a Russian passage in English.
//! The hosted model wrote *every* gist of one English chapter in Chinese, in four runs of five.
//! Neither was detected; `ingest` staged both without remark.
//!
//! This module is the detector, and it is deliberately **script** and not language. Telling
//! Russian from Ukrainian needs a model or a word list; telling Cyrillic from Latin needs a
//! table of code points, which is cheap, total, and wrong in no cases. Both measured failures
//! cross a script boundary, because the wrong-language answer a model actually gives is in its
//! own dominant tongue rather than in a neighbouring one.
//!
//! It is also **relative to the passage**, not to a declared tag. A `LangTag` would have been
//! the obvious comparand and would have measured nothing: `View::lang` defaults to `en` and
//! `ingest` never sets it, so every store this check was written for declares English whatever
//! it holds. The passage is the thing we are certain about — it is the input — so the question
//! asked here is the one `LangPolicy::Source` actually states: *is the gist in the script the
//! passage was in?*
//!
//! **What it does not check.** That the gist is in the right *language*, only the right script:
//! a Serbian gist of a Russian passage passes, both being Cyrillic. That a script this build
//! cannot name is the right one — if either side has no named majority script the comparison is
//! unevaluable and nothing is reported, on `SMY-W025`'s principle that a bound nobody can
//! evaluate is not a bound that passed. And it says nothing about translation quality, which is
//! a judgement and belongs to `attest`.

use crate::{canonical_uid, Code, Diagnostic, UnitCore};

/// A writing system this build can name.
///
/// CJK is one script rather than Han, Kana and Hangul separately, which is a judgement and not
/// an oversight. Japanese prose mixes kana and han freely, so splitting them would leave most
/// Japanese text with no majority script and make the check silent on it — the failure mode
/// this module exists to avoid. The estimator's own classes (`smysl/content/1`) draw the line
/// in the same place, for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Script {
    Latin,
    Cyrillic,
    Greek,
    Cjk,
    Arabic,
    Hebrew,
    Devanagari,
}

impl Script {
    /// Every script this build names, in declaration order.
    pub const ALL: &'static [Script] = &[
        Script::Latin,
        Script::Cyrillic,
        Script::Greek,
        Script::Cjk,
        Script::Arabic,
        Script::Hebrew,
        Script::Devanagari,
    ];

    /// The name a diagnostic prints.
    pub const fn as_str(self) -> &'static str {
        match self {
            Script::Latin => "Latin",
            Script::Cyrillic => "Cyrillic",
            Script::Greek => "Greek",
            Script::Cjk => "CJK",
            Script::Arabic => "Arabic",
            Script::Hebrew => "Hebrew",
            Script::Devanagari => "Devanagari",
        }
    }
}

impl core::fmt::Display for Script {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.pad(self.as_str())
    }
}

/// The script of one character, or `None` when it carries no script.
///
/// Digits, punctuation, spaces and symbols return `None`: they occur in every script and a
/// count that included them would be dominated by whichever text had more commas. `None` is
/// also returned for a letter in a script this build does not name — [`Scripts::of`] counts
/// those separately, because "a script we cannot name" and "no script at all" are different
/// facts and only one of them makes a comparison unevaluable.
pub const fn script_of(c: char) -> Option<Script> {
    // Range patterns rather than `RangeInclusive::contains`, which is not const-stable —
    // the same constraint `estimate::Class::of` works under.
    match c as u32 {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Some(Script::Latin),
        0x400..=0x52F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => {
            Some(Script::Cyrillic)
        }
        0x370..=0x3FF | 0x1F00..=0x1FFF => Some(Script::Greek),
        0x3040..=0x30FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xAC00..=0xD7AF
        | 0x20000..=0x2FA1F => Some(Script::Cjk),
        0x600..=0x6FF | 0x750..=0x77F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Some(Script::Arabic),
        0x590..=0x5FF => Some(Script::Hebrew),
        0x900..=0x97F => Some(Script::Devanagari),
        _ => None,
    }
}

/// How a text's letters divide between the scripts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scripts {
    counts: [u32; 7],
    /// Letters in a script this build does not name.
    unnamed: u32,
}

impl Scripts {
    /// Count the letters of a text by script.
    pub fn of(text: &str) -> Scripts {
        let mut s = Scripts::default();
        for c in text.chars() {
            match script_of(c) {
                Some(script) => s.counts[script as usize] += 1,
                // `is_alphabetic` is what separates a letter from punctuation here, so a
                // Devanagari digit or an ideographic comma counts towards neither side.
                None if c.is_alphabetic() => s.unnamed += 1,
                None => {}
            }
        }
        s
    }

    /// Letters in one named script.
    pub fn count(&self, s: Script) -> u32 {
        self.counts[s as usize]
    }

    /// Letters in a script this build does not name.
    pub const fn unnamed(&self) -> u32 {
        self.unnamed
    }

    /// Every letter, named script or not. Punctuation, digits and spaces are not letters.
    pub fn letters(&self) -> u32 {
        self.counts.iter().sum::<u32>() + self.unnamed
    }

    /// One named script's share of the letters, in whole percent, rounded down.
    ///
    /// Integer percent rather than a float: this number reaches a diagnostic message, and a
    /// diagnostic that reads differently on two targets is a reproducibility defect.
    pub fn percent(&self, s: Script) -> u32 {
        let total = self.letters();
        if total == 0 {
            return 0;
        }
        self.count(s) * 100 / total
    }

    /// The named script holding a strict majority of the letters, if one does.
    ///
    /// `None` covers two cases on purpose, because the caller treats them alike: a genuinely
    /// mixed text, and one written mostly in a script this build cannot name. In both, there is
    /// no script here to compare another text against.
    pub fn dominant(&self) -> Option<Script> {
        let total = self.letters();
        if total == 0 {
            return None;
        }
        Script::ALL
            .iter()
            .copied()
            .find(|s| self.count(*s) * 2 > total)
    }
}

/// Letters a text needs before it is worth asking what script it is in.
///
/// Below this a majority is one or two characters, and a gist like `OK` or `p95` would be
/// reported as Latin against a Russian passage on the strength of nothing. Eight is a floor
/// chosen rather than measured: it is two short words, below which the quantity being
/// percentaged is too small for a percentage to mean anything. Nothing in the S0 corpus
/// pins it, so it is stated here as the arbitrary part of the check.
pub const MIN_LETTERS: u32 = 8;

/// The share below which the passage's script counts as *absent* from a unit, in percent.
///
/// Not a simple "which script leads", and the difference matters. A Russian gist that names an
/// English error — `Сервер вернул 500 Internal Server Error` — is 19 Latin letters against 13
/// Cyrillic, so the leading script is the wrong one and the gist is perfectly good Russian. A
/// borrowed term is normal; what the spike measured was total, gists with no Cyrillic in them
/// at all. So the test is whether the expected script has nearly vanished, which flags the
/// measured failure and leaves loanwords alone.
pub const ABSENT_PERCENT: u32 = 25;

/// Check every unit in a batch against the script of the passage it was drawn from.
///
/// One diagnostic per unit at most, naming each field that is in the wrong script, so a unit
/// whose gist and body are both wrong is one finding rather than two.
///
/// A warning and not an error, which is the whole of its claim to being cheap. Under `ingest`
/// an error buys a repair turn, and repair turns are calls: making this an error would spend a
/// second call on every gist with a Latin loanword in it, on the strength of a code-point
/// table. A warning tells the operator which chunks to look at and costs nothing.
pub fn verify(units: &[UnitCore], source: &str) -> Vec<Diagnostic> {
    let Some(expected) = Scripts::of(source).dominant() else {
        // The passage has no majority script, so there is nothing to hold the units to.
        return Vec::new();
    };

    let mut out = Vec::new();
    for u in units {
        let mut wrong: Vec<(&str, Scripts)> = Vec::new();
        let mut fields: Vec<(&str, &str)> = vec![("gist", u.gist.as_str())];
        if let Some(b) = u.body.as_deref() {
            fields.push(("body", b));
        }
        for (name, text) in fields {
            let s = Scripts::of(text);
            if s.letters() >= MIN_LETTERS && s.percent(expected) < ABSENT_PERCENT {
                wrong.push((name, s));
            }
        }
        if wrong.is_empty() {
            continue;
        }

        let detail = wrong
            .iter()
            .map(|(name, s)| {
                let found = match s.dominant() {
                    Some(d) => d.to_string(),
                    None => "no single script".to_string(),
                };
                format!("{name} is {found} ({}% {expected})", s.percent(expected))
            })
            .collect::<Vec<_>>()
            .join(", ");

        out.push(
            Diagnostic::on(Code::W436, canonical_uid(u))
                .with_message(format!(
                    "the passage is {expected} but {detail}, which `source` policy does not allow"
                ))
                .with_suggestion(
                    "write each unit in the language of its passage, or re-run with a model that does",
                ),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KernelType, Status, Subject, UnitCoreBuilder};

    fn unit(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .expect("a gist and a status are all a claim needs")
    }

    fn unit_with_body(gist: &str, body: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .body(body)
            .build()
            .expect("a gist, a body and a status are all a claim needs")
    }

    const RU: &str = "Сервер перестал отвечать на запросы в три часа ночи по московскому времени.";
    const EN: &str = "The server stopped answering requests at three in the morning.";
    const EL: &str = "Ο διακομιστής σταμάτησε να απαντά σε αιτήματα στις τρεις το πρωί.";
    const ZH: &str = "服务器在凌晨三点停止响应请求。";

    /// `counts` is indexed by `Script as usize`, so the array and `ALL` have to stay the same
    /// length. A script added to the enum without widening the array is an out-of-bounds write,
    /// and this is the assertion that says so at test time rather than at run time.
    #[test]
    fn every_named_script_has_a_counter() {
        assert_eq!(Script::ALL.len(), 7);
        let s = Scripts::default();
        for script in Script::ALL {
            assert_eq!(s.count(*script), 0, "{script} has no counter");
        }
    }

    #[test]
    fn letters_are_counted_and_punctuation_is_not() {
        let s = Scripts::of("abc, 123! —");
        assert_eq!(s.count(Script::Latin), 3);
        assert_eq!(s.letters(), 3, "digits, commas and dashes are not letters");
    }

    #[test]
    fn each_sample_passage_has_the_script_it_is_written_in() {
        for (text, want) in [
            (EN, Script::Latin),
            (RU, Script::Cyrillic),
            (EL, Script::Greek),
            (ZH, Script::Cjk),
        ] {
            assert_eq!(Scripts::of(text).dominant(), Some(want), "{text}");
        }
    }

    /// An unnamed script is counted, and it keeps a named one from claiming a majority it
    /// does not have — which is what makes the comparison unevaluable rather than wrong.
    #[test]
    fn a_script_this_build_cannot_name_is_counted_separately() {
        let s = Scripts::of("ꦗꦮ ꦅ");
        assert_eq!(s.unnamed(), s.letters(), "all of it is unnamed");
        assert_eq!(s.dominant(), None);
        for script in Script::ALL {
            assert_eq!(s.count(*script), 0);
        }
    }

    // --- the two failures the spike measured --------------------------------

    /// The local model's failure: 78.6% of its Russian gists came back in Latin script.
    #[test]
    fn a_latin_gist_of_a_russian_passage_is_w436() {
        let d = verify(&[unit("The server stopped answering requests")], RU);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, Code::W436);
        assert!(d[0].message.contains("Cyrillic"), "{}", d[0].message);
        assert!(d[0].message.contains("gist is Latin"), "{}", d[0].message);
        assert!(d[0].suggestion.is_some());
    }

    /// The hosted model's failure: every gist of one English chapter came back in Chinese,
    /// in four runs of five.
    #[test]
    fn a_chinese_gist_of_an_english_passage_is_w436() {
        let d = verify(&[unit("服务器停止响应请求并且没有恢复")], EN);
        assert_eq!(d.len(), 1);
        assert!(
            d[0].message.contains("Latin but gist is CJK"),
            "{}",
            d[0].message
        );
    }

    #[test]
    fn a_gist_in_the_passages_own_script_is_clean() {
        for (passage, gist) in [
            (RU, "Сервер перестал отвечать"),
            (EN, "The server stopped answering"),
            (EL, "Ο διακομιστής σταμάτησε"),
            (ZH, "服务器停止响应"),
        ] {
            assert!(verify(&[unit(gist)], passage).is_empty(), "{gist}");
        }
    }

    // --- what must not fire -------------------------------------------------

    /// The false positive this check is shaped to avoid. A Russian gist naming an English
    /// error has more Latin letters than Cyrillic ones, so "which script leads" would flag
    /// perfectly good Russian. `ABSENT_PERCENT` asks whether Cyrillic has *vanished*.
    #[test]
    fn a_russian_gist_with_an_english_loanword_is_not_flagged() {
        let gist = "Сервер вернул 500 Internal Server Error";
        let s = Scripts::of(gist);
        assert_eq!(
            s.dominant(),
            Some(Script::Latin),
            "the leading script really is the wrong one"
        );
        assert!(
            s.percent(Script::Cyrillic) >= ABSENT_PERCENT,
            "but Cyrillic is {}%, not absent",
            s.percent(Script::Cyrillic)
        );
        assert!(verify(&[unit(gist)], RU).is_empty());
    }

    /// Short strings have no script worth naming. Without the floor, `OK` is Latin and a
    /// Russian passage would be told its gist was in the wrong language.
    #[test]
    fn a_gist_too_short_to_have_a_script_is_not_flagged() {
        for gist in ["OK", "p95", "500", "ERR 42"] {
            assert!(
                verify(&[unit(gist)], RU).is_empty(),
                "{gist} is under the {MIN_LETTERS}-letter floor"
            );
        }
        // The control: the same shape, long enough, does fire.
        assert_eq!(verify(&[unit("OK and nothing else happened")], RU).len(), 1);
    }

    /// A passage with no majority script holds its units to nothing, on `SMY-W025`'s
    /// principle: unevaluable is not the same as passed, and neither is it a breach.
    #[test]
    fn a_passage_with_no_majority_script_reports_nothing() {
        let mixed = "abc абв αβγ 漢字 ꦗꦮ";
        assert_eq!(Scripts::of(mixed).dominant(), None);
        assert!(verify(&[unit("The server stopped answering requests")], mixed).is_empty());
        assert!(verify(&[unit("服务器停止响应请求并且没有恢复")], mixed).is_empty());
    }

    #[test]
    fn an_empty_passage_reports_nothing() {
        assert!(verify(&[unit("The server stopped answering requests")], "").is_empty());
        assert!(verify(&[unit("The server stopped answering requests")], "123 !?").is_empty());
    }

    // --- the body, and one finding per unit ---------------------------------

    #[test]
    fn the_body_is_checked_too() {
        let u = unit_with_body(
            "Сервер перестал отвечать",
            "The server stopped answering requests at three in the morning.",
        );
        let d = verify(&[u], RU);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("body is Latin"), "{}", d[0].message);
        assert!(!d[0].message.contains("gist is"), "the gist was fine");
    }

    /// Both fields wrong is one finding naming both, not two findings about one unit.
    #[test]
    fn a_unit_wrong_in_both_fields_is_one_diagnostic() {
        let u = unit_with_body(
            "The server stopped answering",
            "It had not recovered by the time anyone looked at it.",
        );
        let d = verify(&[u], RU);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("gist is Latin"), "{}", d[0].message);
        assert!(d[0].message.contains("body is Latin"), "{}", d[0].message);
    }

    #[test]
    fn each_unit_in_a_batch_is_judged_on_its_own() {
        let units = vec![
            unit("Сервер перестал отвечать на запросы"),
            unit("The server stopped answering requests"),
            unit("База данных осталась доступной"),
        ];
        let d = verify(&units, RU);
        assert_eq!(d.len(), 1, "only the Latin one is wrong");
        assert_eq!(d[0].subject, Subject::Unit(canonical_uid(&units[1])));
    }

    #[test]
    fn an_empty_batch_is_clean() {
        assert!(verify(&[], RU).is_empty());
    }

    /// The percentage reaches a diagnostic message, so it has to be the same number
    /// everywhere. Integer division, rounded down, and never a divide by zero.
    #[test]
    fn percent_is_integer_and_safe_on_an_empty_text() {
        assert_eq!(Scripts::of("").percent(Script::Latin), 0);
        assert_eq!(Scripts::of("abcd").percent(Script::Latin), 100);
        // 1 of 3 is 33, rounded down from 33.33.
        assert_eq!(Scripts::of("aбв").percent(Script::Latin), 33);
    }
}
