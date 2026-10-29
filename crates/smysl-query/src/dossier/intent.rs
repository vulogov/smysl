//! Intent: a closed rule table per tier-1 language (SMYSL-2.5 §3.13 step 2).
//!
//! A rule is a prefix, an optional infix and an optional suffix over the **folded** question —
//! lowercased character by character, with the outer `¿`/`¡` and the closing `?`/`!`/`.`
//! removed and runs of whitespace made one space. What lies between the prefix and the suffix
//! is the *topic*, cut from the question as written, so `what is Naomi` keeps `Naomi`'s capital
//! and a subject lookup sees the name a person typed.
//!
//! **The longest match wins**, prefix, infix and suffix together, and a tie goes to the table's
//! order. That is the whole disambiguation rule, and it is why the table needs no ordering
//! discipline of its own: `what is the difference between` (compare) beats `what is` (define)
//! because it is longer, not because somebody remembered to list it first. A rule list that
//! depended on its order would be a rule list one insertion away from a different classifier.
//!
//! No rule matching is `SMY-E468`, and `--intent` names one. There is no model classifier
//! (OQ-22, decided): the rules are pure, which is what makes a dossier a function of its
//! question, and every miss measured was a spelling or a verb a rule can write.
//!
//! Commas are folded away and a typographic apostrophe is the typed one, so a rule is written
//! once for `wer sagte, …` and `wer sagte …`, and for `qu’est-ce que` and `qu'est-ce que`.

use std::fmt;

use smysl_core::diag::{Code, Diagnostic};

/// The eight intents (SMYSL-2.5 §3.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Intent {
    Define,
    Explain,
    Verify,
    Attribute,
    Trace,
    Locate,
    Depend,
    Compare,
}

impl Intent {
    pub const ALL: &'static [Intent] = &[
        Intent::Define,
        Intent::Explain,
        Intent::Verify,
        Intent::Attribute,
        Intent::Trace,
        Intent::Locate,
        Intent::Depend,
        Intent::Compare,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Intent::Define => "define",
            Intent::Explain => "explain",
            Intent::Verify => "verify",
            Intent::Attribute => "attribute",
            Intent::Trace => "trace",
            Intent::Locate => "locate",
            Intent::Depend => "depend",
            Intent::Compare => "compare",
        }
    }

    pub fn parse(s: &str) -> Option<Intent> {
        Intent::ALL.iter().copied().find(|i| i.as_str() == s)
    }
}

impl fmt::Display for Intent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// The languages the table covers, in the order a tie between them is broken.
pub const LANGS: &[&str] = &["en", "de", "es", "fr", "ru"];

/// One rule: `prefix TOPIC infix TOPIC suffix`, over the folded question.
///
/// With an infix the topic is the whole question, since a comparison has two subjects and the
/// subject search looks for a name anywhere in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub lang: &'static str,
    pub intent: Intent,
    pub prefix: &'static str,
    pub infix: &'static str,
    pub suffix: &'static str,
}

impl Rule {
    /// `<lang>/<intent>/<prefix…suffix>`, which is what a recipe and `--explain` name.
    pub fn id(&self) -> String {
        let mut words = vec![self.prefix.trim()];
        if !self.infix.is_empty() {
            words.push("…");
            words.push(self.infix.trim());
        }
        words.push("…");
        if !self.suffix.is_empty() {
            words.push(self.suffix.trim());
        }
        format!("{}/{}/{}", self.lang, self.intent, words.join(" ").trim())
    }

    fn weight(&self) -> usize {
        self.prefix.len() + self.infix.len() + self.suffix.len()
    }
}

const fn r(lang: &'static str, intent: Intent, prefix: &'static str) -> Rule {
    Rule {
        lang,
        intent,
        prefix,
        infix: "",
        suffix: "",
    }
}

const fn rs(
    lang: &'static str,
    intent: Intent,
    prefix: &'static str,
    suffix: &'static str,
) -> Rule {
    Rule {
        lang,
        intent,
        prefix,
        infix: "",
        suffix,
    }
}

const fn ri(lang: &'static str, intent: Intent, infix: &'static str) -> Rule {
    Rule {
        lang,
        intent,
        prefix: "",
        infix,
        suffix: "",
    }
}

use Intent::{Attribute, Compare, Define, Depend, Explain, Locate, Trace, Verify};

/// The table. Every prefix ends in a space or an apostrophe, so `whyever` is not `why`.
pub const RULES: &[Rule] = &[
    // --- en ----------------------------------------------------------------------------------
    r("en", Define, "what is "),
    r("en", Define, "what are "),
    r("en", Define, "what was "),
    r("en", Define, "who is "),
    r("en", Define, "who was "),
    r("en", Define, "who were "),
    r("en", Define, "define "),
    r("en", Define, "tell me about "),
    rs("en", Define, "what does ", " mean"),
    r("en", Explain, "why "),
    r("en", Explain, "why did "),
    r("en", Explain, "why does "),
    r("en", Explain, "why is "),
    r("en", Explain, "why was "),
    r("en", Explain, "how come "),
    r("en", Explain, "explain "),
    r("en", Explain, "what caused "),
    r("en", Verify, "is it true that "),
    r("en", Verify, "is there evidence that "),
    r("en", Verify, "verify that "),
    r("en", Verify, "did "),
    r("en", Verify, "does "),
    r("en", Attribute, "who said "),
    r("en", Attribute, "who said that "),
    r("en", Attribute, "who says "),
    r("en", Attribute, "who claims "),
    r("en", Attribute, "who claimed "),
    r("en", Attribute, "who believes "),
    r("en", Attribute, "according to whom "),
    rs("en", Trace, "where did ", " come from"),
    rs("en", Trace, "how did ", " develop"),
    r("en", Trace, "what is the history of "),
    r("en", Trace, "history of "),
    r("en", Trace, "origin of "),
    r("en", Trace, "trace "),
    r("en", Locate, "where is "),
    r("en", Locate, "where are "),
    r("en", Locate, "where was "),
    rs("en", Locate, "where is ", " mentioned"),
    rs("en", Locate, "where are ", " mentioned"),
    rs("en", Locate, "where does ", " appear"),
    rs("en", Locate, "where do ", " appear"),
    r("en", Depend, "what depends on "),
    r("en", Depend, "what relies on "),
    r("en", Depend, "what rests on "),
    r("en", Depend, "what follows from "),
    r("en", Compare, "compare "),
    r("en", Compare, "what is the difference between "),
    r("en", Compare, "difference between "),
    ri("en", Compare, " vs "),
    ri("en", Compare, " versus "),
    // --- de ----------------------------------------------------------------------------------
    r("de", Define, "was ist "),
    r("de", Define, "was sind "),
    r("de", Define, "wer ist "),
    r("de", Define, "wer war "),
    r("de", Define, "definiere "),
    r("de", Define, "definition von "),
    r("de", Define, "definition des "),
    r("de", Define, "definition der "),
    r("de", Explain, "warum "),
    r("de", Explain, "wieso "),
    r("de", Explain, "weshalb "),
    r("de", Explain, "erkläre "),
    r("de", Verify, "stimmt es dass "),
    r("de", Verify, "ist es wahr dass "),
    r("de", Attribute, "wer sagte "),
    r("de", Attribute, "wer hat gesagt "),
    r("de", Attribute, "wer behauptet "),
    r("de", Attribute, "laut wem "),
    r("de", Trace, "woher kommt "),
    r("de", Trace, "woher kommen "),
    r("de", Trace, "woher stammt "),
    r("de", Trace, "woher stammen "),
    r("de", Trace, "geschichte von "),
    r("de", Trace, "geschichte der "),
    r("de", Trace, "geschichte des "),
    r("de", Trace, "herkunft von "),
    r("de", Trace, "herkunft der "),
    r("de", Trace, "herkunft des "),
    r("de", Locate, "wo ist "),
    r("de", Locate, "wo steht "),
    rs("de", Locate, "wo wird ", " erwähnt"),
    rs("de", Locate, "wo werden ", " erwähnt"),
    rs("de", Locate, "wo kommt ", " vor"),
    rs("de", Locate, "wo kommen ", " vor"),
    rs("de", Depend, "was hängt von ", " ab"),
    rs("de", Depend, "was hängt vom ", " ab"),
    rs("de", Depend, "was hängt von der ", " ab"),
    rs("de", Depend, "was hängen von ", " ab"),
    r("de", Depend, "was folgt aus "),
    r("de", Compare, "vergleiche "),
    r("de", Compare, "was ist der unterschied zwischen "),
    r("de", Compare, "unterschied zwischen "),
    // --- es ----------------------------------------------------------------------------------
    r("es", Define, "qué es "),
    r("es", Define, "que es "),
    r("es", Define, "qué son "),
    r("es", Define, "quién es "),
    r("es", Define, "quien es "),
    r("es", Define, "quién era "),
    r("es", Define, "definición de "),
    r("es", Define, "definición del "),
    r("es", Explain, "por qué "),
    r("es", Explain, "por que "),
    r("es", Explain, "explica "),
    r("es", Verify, "es cierto que "),
    r("es", Verify, "es verdad que "),
    r("es", Attribute, "quién dijo "),
    r("es", Attribute, "quien dijo "),
    r("es", Attribute, "quién afirma "),
    r("es", Attribute, "según quién "),
    r("es", Trace, "de dónde viene "),
    r("es", Trace, "de donde viene "),
    r("es", Trace, "historia de "),
    r("es", Trace, "historia del "),
    r("es", Trace, "origen de "),
    r("es", Trace, "origen del "),
    r("es", Locate, "dónde está "),
    r("es", Locate, "donde está "),
    r("es", Locate, "dónde aparece "),
    r("es", Locate, "dónde aparecen "),
    r("es", Locate, "dónde están "),
    r("es", Depend, "qué depende de "),
    r("es", Depend, "qué depende del "),
    r("es", Depend, "qué dependen de "),
    r("es", Depend, "qué se sigue de "),
    r("es", Depend, "qué se sigue del "),
    r("es", Compare, "compara "),
    r("es", Compare, "cuál es la diferencia entre "),
    r("es", Compare, "diferencia entre "),
    // --- fr ----------------------------------------------------------------------------------
    r("fr", Define, "qu'est-ce que "),
    r("fr", Define, "qu'est-ce qu'"),
    r("fr", Define, "qui est "),
    r("fr", Define, "qui était "),
    r("fr", Define, "définition de "),
    r("fr", Define, "définition du "),
    r("fr", Define, "définition des "),
    r("fr", Explain, "pourquoi "),
    r("fr", Explain, "explique "),
    r("fr", Explain, "expliquez "),
    r("fr", Verify, "est-il vrai que "),
    r("fr", Verify, "est-il vrai qu'"),
    r("fr", Verify, "est-ce vrai que "),
    r("fr", Verify, "est-ce vrai qu'"),
    r("fr", Attribute, "qui a dit "),
    r("fr", Attribute, "qui affirme "),
    r("fr", Attribute, "selon qui "),
    r("fr", Trace, "d'où vient "),
    r("fr", Trace, "histoire de "),
    r("fr", Trace, "histoire du "),
    r("fr", Trace, "histoire des "),
    r("fr", Trace, "origine de "),
    r("fr", Trace, "origine du "),
    r("fr", Trace, "origine des "),
    r("fr", Locate, "où est "),
    r("fr", Locate, "où se trouve "),
    r("fr", Locate, "où sont "),
    r("fr", Locate, "où apparaît "),
    r("fr", Depend, "qu'est-ce qui dépend de "),
    r("fr", Depend, "qu'est-ce qui dépend du "),
    r("fr", Depend, "qu'est-ce qui dépend des "),
    r("fr", Depend, "qui dépend de "),
    r("fr", Depend, "qui dépend du "),
    r("fr", Compare, "compare "),
    r("fr", Compare, "comparez "),
    r("fr", Compare, "quelle est la différence entre "),
    r("fr", Compare, "différence entre "),
    // --- ru ----------------------------------------------------------------------------------
    r("ru", Define, "что такое "),
    r("ru", Define, "кто такой "),
    r("ru", Define, "кто такая "),
    r("ru", Define, "кто такие "),
    r("ru", Define, "кто был "),
    r("ru", Define, "кто была "),
    r("ru", Define, "определение "),
    r("ru", Explain, "почему "),
    r("ru", Explain, "зачем "),
    r("ru", Explain, "отчего "),
    r("ru", Explain, "объясни "),
    r("ru", Verify, "правда ли что "),
    r("ru", Verify, "верно ли что "),
    r("ru", Verify, "действительно ли "),
    r("ru", Attribute, "кто сказал "),
    r("ru", Attribute, "кто сказал что "),
    r("ru", Attribute, "кто утверждает "),
    r("ru", Attribute, "по чьим словам "),
    r("ru", Trace, "откуда "),
    r("ru", Trace, "история "),
    r("ru", Trace, "происхождение "),
    r("ru", Locate, "где "),
    r("ru", Locate, "где упоминается "),
    r("ru", Depend, "что зависит от "),
    r("ru", Depend, "что следует из "),
    r("ru", Compare, "сравни "),
    r("ru", Compare, "сравните "),
    r("ru", Compare, "чем отличается "),
    r("ru", Compare, "разница между "),
];

/// What the table made of a question.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Classified {
    pub intent: Intent,
    pub lang: &'static str,
    /// [`Rule::id`] of the rule that matched.
    pub rule: String,
    /// The text between the rule's parts, as the question wrote it.
    pub topic: String,
}

/// The question folded, with a map from each folded byte back to the question's own.
struct Folded {
    text: String,
    /// `origin[i]` is the byte in the original where the character producing folded byte `i`
    /// starts; one entry past the end maps the end.
    origin: Vec<usize>,
}

fn fold(question: &str) -> Folded {
    let trimmed_start = question.len()
        - question
            .trim_start_matches(|c: char| c.is_whitespace() || c == '¿' || c == '¡')
            .len();
    let body = question.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '?' | '!' | '.' | '？' | '！' | '。')
    });
    let mut text = String::new();
    let mut origin = Vec::new();
    let mut last_space = true;
    let end = body.len().max(trimmed_start);
    for (i, c) in question[trimmed_start..end].char_indices() {
        let at = trimmed_start + i;
        if c.is_whitespace() {
            if last_space {
                continue;
            }
            text.push(' ');
            origin.push(at);
            last_space = true;
            continue;
        }
        // A comma is punctuation between a rule's verb and its clause (`wer sagte, …`,
        // `stimmt es, dass …`), and never part of a rule: folded away, so one rule reads both.
        if c == ',' {
            continue;
        }
        last_space = false;
        let before = text.len();
        // One apostrophe: a typographic one is the same elision as a typed one.
        let c = if c == '’' { '\'' } else { c };
        text.push(crate::fold::lower(c));
        origin.extend(std::iter::repeat_n(at, text.len() - before));
    }
    origin.push(end);
    Folded { text, origin }
}

impl Folded {
    /// The original text behind folded bytes `start..end`, trimmed.
    fn original<'q>(&self, question: &'q str, start: usize, end: usize) -> &'q str {
        let a = self.origin[start];
        // `end` is a folded byte boundary; the original slice ends where the next folded
        // character starts, which is the start of the character that produced byte `end`.
        let b = self.origin[end];
        question[a..b].trim()
    }
}

/// Classify a question. `lang` narrows the table to one language; `None` tries all five.
pub fn classify(question: &str, lang: Option<&str>) -> Result<Classified, Diagnostic> {
    let folded = fold(question);
    let f = folded.text.as_str();
    let mut best: Option<(&Rule, usize, usize, usize)> = None; // rule, weight, topic start, end
    for rule in RULES {
        if lang.is_some_and(|l| l != rule.lang) {
            continue;
        }
        let matched = if !rule.infix.is_empty() {
            f.contains(rule.infix).then_some((0, f.len()))
        } else if f.starts_with(rule.prefix)
            && f.ends_with(rule.suffix)
            && f.len() > rule.prefix.len() + rule.suffix.len()
        {
            Some((rule.prefix.len(), f.len() - rule.suffix.len()))
        } else {
            None
        };
        let Some((start, end)) = matched else {
            continue;
        };
        let w = rule.weight();
        // Strictly longer wins, so a tie keeps the earlier rule: the table's order, and
        // `LANGS`'s order between languages, since the table is written in that order.
        if best.is_none_or(|(_, bw, _, _)| w > bw) {
            best = Some((rule, w, start, end));
        }
    }
    let Some((rule, _, start, end)) = best else {
        return Err(Diagnostic::new(Code::E468).with_message(format!(
            "`{}` matches no intent rule{}; name the intent with --intent ({})",
            question.trim(),
            lang.map(|l| format!(" for `{l}`")).unwrap_or_default(),
            Intent::ALL
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    let topic = folded.original(question, start, end).to_string();
    if topic.is_empty() {
        return Err(Diagnostic::new(Code::E468).with_message(format!(
            "`{}` matches `{}` and leaves nothing to ask about",
            question.trim(),
            rule.id()
        )));
    }
    Ok(Classified {
        intent: rule.intent,
        lang: rule.lang,
        rule: rule.id(),
        topic,
    })
}

/// The topic with no rule applied: the whole question, folded only at its ends. What `--intent`
/// classifies a question as when it names the intent itself.
pub fn untouched(question: &str, intent: Intent) -> Classified {
    let folded = fold(question);
    Classified {
        intent,
        lang: "",
        rule: format!("--intent {intent}"),
        topic: folded.original(question, 0, folded.text.len()).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prefix_ends_at_a_word_boundary() {
        for rule in RULES {
            if !rule.prefix.is_empty() {
                assert!(
                    rule.prefix.ends_with(' ')
                        || rule.prefix.ends_with('\'')
                        || rule.prefix.ends_with('’'),
                    "{}",
                    rule.id()
                );
            }
            assert_eq!(rule.prefix, crate::fold::fold(rule.prefix), "{}", rule.id());
        }
    }

    #[test]
    fn the_table_is_written_in_tie_break_order() {
        let order: Vec<&str> = RULES.iter().map(|r| r.lang).collect();
        let mut sorted = order.clone();
        sorted.sort_by_key(|l| LANGS.iter().position(|x| x == l));
        assert_eq!(order, sorted);
    }

    #[test]
    fn the_topic_keeps_its_capitals_and_loses_its_punctuation() {
        let c = classify("  What is   Naomi?  ", None).unwrap();
        assert_eq!(
            (c.intent, c.lang, c.topic.as_str()),
            (Intent::Define, "en", "Naomi")
        );
        let c = classify("¿Quién es Noemí?", None).unwrap();
        assert_eq!(
            (c.intent, c.lang, c.topic.as_str()),
            (Intent::Define, "es", "Noemí")
        );
    }

    #[test]
    fn longer_rules_win_whatever_their_order() {
        let c = classify("What is the difference between Ruth and Orpah?", None).unwrap();
        assert_eq!(c.intent, Intent::Compare);
        let c = classify("What is the history of Moab", None).unwrap();
        assert_eq!(c.intent, Intent::Trace);
        let c = classify("Where is Bethlehem mentioned?", None).unwrap();
        assert_eq!((c.intent, c.topic.as_str()), (Intent::Locate, "Bethlehem"));
    }

    #[test]
    fn a_prefix_alone_is_not_a_question() {
        assert_eq!(classify("What is?", None).unwrap_err().code, Code::E468);
        assert_eq!(classify("whyever not", None).unwrap_err().code, Code::E468);
    }
}
