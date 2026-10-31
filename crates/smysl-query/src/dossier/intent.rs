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
    r("en", Define, "what exactly is "),
    r("en", Define, "what exactly was "),
    r("en", Define, "who exactly is "),
    r("en", Define, "who exactly was "),
    r("en", Define, "what precisely is "),
    r("en", Define, "what precisely was "),
    r("en", Define, "describe "),
    rs("en", Define, "what does ", " mean"),
    r("en", Explain, "why "),
    r("en", Explain, "why did "),
    r("en", Explain, "why does "),
    r("en", Explain, "why is "),
    r("en", Explain, "why was "),
    r("en", Explain, "how come "),
    r("en", Explain, "explain "),
    r("en", Explain, "what caused "),
    r("en", Explain, "what made "),
    r("en", Verify, "is it true that "),
    r("en", Verify, "is there evidence that "),
    r("en", Verify, "is there any evidence that "),
    r("en", Verify, "is there proof that "),
    r("en", Verify, "verify that "),
    r("en", Verify, "did "),
    r("en", Verify, "does "),
    r("en", Attribute, "who said "),
    r("en", Attribute, "who said that "),
    r("en", Attribute, "who says "),
    r("en", Attribute, "who claims "),
    r("en", Attribute, "who claimed "),
    r("en", Attribute, "who believes "),
    r("en", Attribute, "who believed "),
    r("en", Attribute, "who believed in "),
    r("en", Attribute, "who believed that "),
    r("en", Attribute, "who doubted "),
    r("en", Attribute, "according to whom "),
    rs("en", Trace, "where did ", " come from"),
    rs("en", Trace, "how did ", " develop"),
    rs("en", Trace, "how did ", " arise"),
    rs("en", Trace, "where did ", " come from originally"),
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
    r("en", Locate, "where do the texts mention "),
    r("en", Locate, "where do the sources mention "),
    r("en", Depend, "what depends on "),
    r("en", Depend, "what relies on "),
    r("en", Depend, "what rests on "),
    r("en", Depend, "what follows from "),
    r("en", Depend, "what depended on "),
    r("en", Depend, "what relied on "),
    r("en", Depend, "what was built on "),
    r("en", Depend, "what is built on "),
    r("en", Compare, "compare "),
    r("en", Compare, "what is the difference between "),
    r("en", Compare, "difference between "),
    ri("en", Compare, " vs "),
    ri("en", Compare, " versus "),
    ri("en", Compare, " compare with "),
    // --- de ----------------------------------------------------------------------------------
    r("de", Define, "was ist "),
    r("de", Define, "was sind "),
    r("de", Define, "wer ist "),
    r("de", Define, "wer war "),
    r("de", Define, "was war "),
    r("de", Define, "was genau ist "),
    r("de", Define, "was genau war "),
    r("de", Define, "wer genau ist "),
    r("de", Define, "wer genau war "),
    r("de", Define, "was versteht man unter "),
    r("de", Define, "definiere "),
    r("de", Define, "definition von "),
    r("de", Define, "definition des "),
    r("de", Define, "definition der "),
    r("de", Explain, "warum "),
    r("de", Explain, "wieso "),
    r("de", Explain, "weshalb "),
    r("de", Explain, "erkläre "),
    r("de", Explain, "aus welchem grund "),
    r("de", Verify, "stimmt es dass "),
    r("de", Verify, "ist es wahr dass "),
    r("de", Verify, "gibt es belege dass "),
    r("de", Verify, "gibt es belege dafür dass "),
    r("de", Verify, "gibt es beweise dass "),
    r("de", Verify, "gibt es beweise dafür dass "),
    r("de", Verify, "gibt es einen nachweis dass "),
    r("de", Verify, "gibt es nachweise dass "),
    r("de", Verify, "hat "),
    r("de", Verify, "haben "),
    r("de", Verify, "hatte "),
    r("de", Attribute, "wer sagte "),
    r("de", Attribute, "wer hat gesagt "),
    r("de", Attribute, "wer behauptet "),
    r("de", Attribute, "wer behauptete "),
    r("de", Attribute, "wer bezweifelte "),
    r("de", Attribute, "laut wem "),
    r("de", Attribute, "wer glaubte an "),
    r("de", Attribute, "wer glaubte dass "),
    r("de", Trace, "woher kommt "),
    r("de", Trace, "woher kommen "),
    r("de", Trace, "woher stammt "),
    r("de", Trace, "woher stammen "),
    r("de", Trace, "woher kam "),
    r("de", Trace, "woher kamen "),
    r("de", Trace, "wie entstand "),
    r("de", Trace, "wie entstanden "),
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
    rs("de", Locate, "wo taucht ", " auf"),
    rs("de", Locate, "wo tauchen ", " auf"),
    rs("de", Locate, "wo ist von ", " die rede"),
    rs("de", Depend, "was hängt von ", " ab"),
    rs("de", Depend, "was hängt vom ", " ab"),
    rs("de", Depend, "was hängt von der ", " ab"),
    rs("de", Depend, "was hängen von ", " ab"),
    r("de", Depend, "was folgt aus "),
    rs("de", Depend, "was hing von ", " ab"),
    rs("de", Depend, "was hing vom ", " ab"),
    rs("de", Depend, "was hing von der ", " ab"),
    rs("de", Depend, "was baute auf ", " auf"),
    rs("de", Depend, "was baut auf ", " auf"),
    r("de", Compare, "vergleiche "),
    r("de", Compare, "was ist der unterschied zwischen "),
    r("de", Compare, "unterschied zwischen "),
    r("de", Compare, "wie unterscheidet sich "),
    // --- es ----------------------------------------------------------------------------------
    r("es", Define, "qué es "),
    r("es", Define, "que es "),
    r("es", Define, "qué son "),
    r("es", Define, "quién es "),
    r("es", Define, "quien es "),
    r("es", Define, "quién era "),
    r("es", Define, "quién fue "),
    r("es", Define, "quien fue "),
    r("es", Define, "qué era "),
    r("es", Define, "que era "),
    r("es", Define, "qué fue "),
    r("es", Define, "qué era exactamente "),
    r("es", Define, "qué es exactamente "),
    r("es", Define, "en qué consiste "),
    r("es", Define, "en qué consistía "),
    r("es", Define, "definición de "),
    r("es", Define, "definición del "),
    r("es", Explain, "por qué "),
    r("es", Explain, "por que "),
    r("es", Explain, "explica "),
    r("es", Explain, "cómo es que "),
    r("es", Verify, "es cierto que "),
    r("es", Verify, "es verdad que "),
    r("es", Verify, "hay pruebas de que "),
    r("es", Verify, "hay evidencia de que "),
    r("es", Verify, "existen pruebas de que "),
    r("es", Attribute, "quién dijo "),
    r("es", Attribute, "quien dijo "),
    r("es", Attribute, "quién afirma "),
    r("es", Attribute, "según quién "),
    r("es", Attribute, "quién creía en "),
    r("es", Attribute, "quien creía en "),
    r("es", Attribute, "quién creía que "),
    r("es", Attribute, "quién dudaba de "),
    r("es", Trace, "de dónde viene "),
    r("es", Trace, "de donde viene "),
    r("es", Trace, "de dónde vino "),
    r("es", Trace, "de donde vino "),
    r("es", Trace, "de dónde venía "),
    r("es", Trace, "cómo surgió "),
    r("es", Trace, "cómo surgieron "),
    r("es", Trace, "historia de "),
    r("es", Trace, "historia del "),
    r("es", Trace, "origen de "),
    r("es", Trace, "origen del "),
    r("es", Locate, "dónde está "),
    r("es", Locate, "donde está "),
    r("es", Locate, "dónde aparece "),
    r("es", Locate, "dónde aparecen "),
    r("es", Locate, "dónde están "),
    r("es", Locate, "dónde se menciona "),
    r("es", Locate, "donde se menciona "),
    r("es", Locate, "dónde se menciona a "),
    r("es", Locate, "dónde se mencionan "),
    r("es", Locate, "dónde se habla de "),
    r("es", Locate, "dónde se habla del "),
    r("es", Locate, "donde se habla de "),
    r("es", Locate, "en qué textos aparece "),
    r("es", Depend, "qué depende de "),
    r("es", Depend, "qué depende del "),
    r("es", Depend, "qué dependen de "),
    r("es", Depend, "qué se sigue de "),
    r("es", Depend, "qué se sigue del "),
    r("es", Depend, "qué dependía de "),
    r("es", Depend, "qué dependía del "),
    r("es", Depend, "qué dependían de "),
    r("es", Depend, "qué se basaba en "),
    r("es", Depend, "qué se basa en "),
    r("es", Compare, "compara "),
    r("es", Compare, "cuál es la diferencia entre "),
    r("es", Compare, "diferencia entre "),
    r("es", Compare, "en qué se diferencia "),
    // --- fr ----------------------------------------------------------------------------------
    r("fr", Define, "qu'est-ce que "),
    r("fr", Define, "qu'est-ce qu'"),
    r("fr", Define, "qui est "),
    r("fr", Define, "qui était "),
    r("fr", Define, "qu'était "),
    r("fr", Define, "qu'était exactement "),
    r("fr", Define, "qui était exactement "),
    r("fr", Define, "qui fut "),
    r("fr", Define, "qu'appelle-t-on "),
    r("fr", Define, "définition de "),
    r("fr", Define, "définition du "),
    r("fr", Define, "définition des "),
    r("fr", Explain, "pourquoi "),
    r("fr", Explain, "explique "),
    r("fr", Explain, "expliquez "),
    r("fr", Explain, "comment se fait-il que "),
    r("fr", Explain, "comment se fait-il qu'"),
    r("fr", Verify, "est-il vrai que "),
    r("fr", Verify, "est-il vrai qu'"),
    r("fr", Verify, "est-ce vrai que "),
    r("fr", Verify, "est-ce vrai qu'"),
    r("fr", Verify, "y a-t-il des preuves que "),
    r("fr", Verify, "y a-t-il des preuves qu'"),
    r("fr", Verify, "existe-t-il des preuves que "),
    r("fr", Verify, "existe-t-il des preuves qu'"),
    r("fr", Verify, "est-ce que "),
    r("fr", Verify, "est-ce qu'"),
    r("fr", Attribute, "qui a dit "),
    r("fr", Attribute, "qui affirme "),
    r("fr", Attribute, "selon qui "),
    r("fr", Attribute, "qui croyait à "),
    r("fr", Attribute, "qui croyait au "),
    r("fr", Attribute, "qui croyait aux "),
    r("fr", Attribute, "qui croyait en "),
    r("fr", Attribute, "qui croyait que "),
    r("fr", Attribute, "qui doutait de "),
    r("fr", Trace, "d'où vient "),
    r("fr", Trace, "d'où venait "),
    r("fr", Trace, "d'où est venu "),
    r("fr", Trace, "d'où est venue "),
    r("fr", Trace, "comment est né "),
    r("fr", Trace, "comment est née "),
    r("fr", Trace, "comment sont nés "),
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
    r("fr", Locate, "où parle-t-on de "),
    r("fr", Locate, "où parle-t-on du "),
    r("fr", Locate, "où parle-t-on des "),
    r("fr", Locate, "dans quels textes apparaît "),
    r("fr", Depend, "qu'est-ce qui dépend de "),
    r("fr", Depend, "qu'est-ce qui dépend du "),
    r("fr", Depend, "qu'est-ce qui dépend des "),
    r("fr", Depend, "qu'est-ce qui dépendait de "),
    r("fr", Depend, "qu'est-ce qui dépendait du "),
    r("fr", Depend, "qu'est-ce qui dépendait des "),
    r("fr", Depend, "qu'est-ce qui reposait sur "),
    r("fr", Depend, "qu'est-ce qui repose sur "),
    r("fr", Depend, "qui dépend de "),
    r("fr", Depend, "qui dépend du "),
    r("fr", Compare, "compare "),
    r("fr", Compare, "comparez "),
    r("fr", Compare, "quelle est la différence entre "),
    r("fr", Compare, "différence entre "),
    ri("fr", Compare, " diffère-t-il de "),
    ri("fr", Compare, " diffère-t-elle de "),
    // --- ru ----------------------------------------------------------------------------------
    r("ru", Define, "что такое "),
    r("ru", Define, "кто такой "),
    r("ru", Define, "кто такая "),
    r("ru", Define, "кто такие "),
    r("ru", Define, "кто был "),
    r("ru", Define, "кто была "),
    r("ru", Define, "кем был "),
    r("ru", Define, "кем была "),
    r("ru", Define, "кем были "),
    r("ru", Define, "что представляет собой "),
    r("ru", Define, "что представлял собой "),
    r("ru", Define, "что представляли собой "),
    r("ru", Define, "определение "),
    r("ru", Explain, "почему "),
    r("ru", Explain, "зачем "),
    r("ru", Explain, "отчего "),
    r("ru", Explain, "объясни "),
    r("ru", Verify, "правда ли что "),
    r("ru", Verify, "верно ли что "),
    r("ru", Verify, "действительно ли "),
    r("ru", Verify, "есть ли доказательства что "),
    r("ru", Verify, "есть ли доказательства того что "),
    r("ru", Verify, "есть ли свидетельства что "),
    r("ru", Attribute, "кто сказал "),
    r("ru", Attribute, "кто сказал что "),
    r("ru", Attribute, "кто утверждает "),
    r("ru", Attribute, "по чьим словам "),
    r("ru", Attribute, "кто утверждал "),
    r("ru", Attribute, "кто утверждал что "),
    r("ru", Attribute, "кто верил в "),
    r("ru", Attribute, "кто верил во "),
    r("ru", Attribute, "кто сомневался в "),
    r("ru", Attribute, "кто верил что "),
    r("ru", Trace, "откуда "),
    r("ru", Trace, "история "),
    r("ru", Trace, "происхождение "),
    r("ru", Trace, "как возник "),
    r("ru", Trace, "как возникла "),
    r("ru", Trace, "как возникло "),
    r("ru", Trace, "как возникли "),
    r("ru", Locate, "где "),
    r("ru", Locate, "где упоминается "),
    r("ru", Depend, "что зависит от "),
    r("ru", Depend, "что следует из "),
    r("ru", Depend, "что зависело от "),
    r("ru", Depend, "что опиралось на "),
    r("ru", Depend, "что опирается на "),
    r("ru", Compare, "сравни "),
    r("ru", Compare, "сравните "),
    r("ru", Compare, "чем отличается "),
    r("ru", Compare, "разница между "),
    ri("ru", Compare, " отличается от "),
];
/// Rule 27: a yes-or-no question as verification, by the verb that opens it or the inversion
/// that marks it. A last-resort tier — read only when no rule of [`RULES`] reads the question,
/// so `why is`, `where is` and `pourquoi … -t-il` keep their readings however short they are.
///
/// An infix here is matched against whole words, as a word's ending when it starts with `-`
/// (French inversion, `existe-t-il`) and as the word itself otherwise (Russian `ли`), and only
/// in a question that does not open with one of [`QUESTION_WORDS`]: `qu'est-il résulté de …`
/// asks what, not whether.
pub const LAST_RESORT: &[Rule] = &[
    r("en", Verify, "is "),
    r("en", Verify, "are "),
    r("en", Verify, "was "),
    r("en", Verify, "were "),
    r("en", Verify, "has "),
    r("en", Verify, "have "),
    r("en", Verify, "had "),
    r("en", Verify, "do "),
    r("en", Verify, "can "),
    r("en", Verify, "could "),
    r("de", Verify, "ist "),
    r("de", Verify, "sind "),
    r("de", Verify, "war "),
    r("de", Verify, "waren "),
    r("de", Verify, "wird "),
    r("de", Verify, "werden "),
    r("de", Verify, "wurde "),
    r("de", Verify, "wurden "),
    r("de", Verify, "kann "),
    r("de", Verify, "konnte "),
    r("es", Verify, "es "),
    r("es", Verify, "son "),
    r("es", Verify, "fue "),
    r("es", Verify, "fueron "),
    r("es", Verify, "se "),
    r("es", Verify, "puede "),
    ri("fr", Verify, "-t-il"),
    ri("fr", Verify, "-t-elle"),
    ri("fr", Verify, "-il"),
    ri("fr", Verify, "-elle"),
    ri("fr", Verify, "-ils"),
    ri("fr", Verify, "-elles"),
    ri("ru", Verify, "ли"),
];

/// The words that open a question asking something other than whether, per language with an
/// infix in [`LAST_RESORT`]; `qu'` matches as the start of the first word.
pub const QUESTION_WORDS: &[(&str, &[&str])] = &[
    (
        "fr",
        &[
            "qu'", "que", "quel", "quelle", "quels", "quelles", "qui", "comment", "où", "pourquoi",
            "combien", "quand",
        ],
    ),
    (
        "ru",
        &[
            "что",
            "кто",
            "как",
            "где",
            "почему",
            "зачем",
            "когда",
            "какой",
            "какая",
            "какое",
            "какие",
            "каков",
            "сколько",
            "откуда",
        ],
    ),
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
/// The question as the rule table reads it: trimmed of its opening and closing punctuation,
/// lowercased, its runs of space made one, its commas dropped and a typographic apostrophe made
/// a typed one. What `ask --explain` shows, so a question no rule reads can be compared with the
/// rules' own spelling.
pub fn folded(question: &str) -> String {
    fold(question).text
}

/// Common short words of each Latin-script tier-1 language, for [`guess_lang`]: articles,
/// question words, auxiliaries and prepositions. A word two languages share counts for both.
///
/// Completed in 1.12.0 by a rule rather than by a question: each language's articles in all
/// their forms (German `einem`, `einer`, `eines`; Spanish `unos`, `unas`), and each word that
/// opens one of its verification rules in [`RULES`] or [`LAST_RESORT`] (English `has`, `can`;
/// German `hat`, `wird`; Spanish `fueron`, `puede`). In-sample for the one question it was
/// wrong on, a German `Hat … an einem …` read as English by its `an`; the yes-or-no and forms
/// sets were written before it and were run under the lists before it.
const FUNCTION_WORDS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "a", "an", "is", "are", "was", "were", "what", "who", "how", "why", "where",
            "which", "did", "does", "do", "of", "to", "in", "and", "about", "for", "from", "has",
            "have", "had", "can", "could",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "ist", "sind", "war",
            "was", "wer", "wie", "warum", "wo", "woher", "welche", "und", "von", "vom", "zu",
            "zum", "mit", "über", "es", "dass", "nicht", "einem", "einer", "eines", "waren",
            "wird", "werden", "wurde", "wurden", "hat", "haben", "hatte", "kann", "konnte",
        ],
    ),
    (
        "es",
        &[
            "el", "la", "los", "las", "un", "una", "es", "son", "fue", "qué", "quién", "cómo",
            "por", "dónde", "cuál", "de", "del", "y", "que", "en", "se", "con", "sobre", "unos",
            "unas", "fueron", "puede",
        ],
    ),
    (
        "fr",
        &[
            "le",
            "la",
            "les",
            "un",
            "une",
            "est",
            "sont",
            "était",
            "qu'est-ce",
            "qui",
            "que",
            "quel",
            "quelle",
            "pourquoi",
            "comment",
            "où",
            "de",
            "du",
            "des",
            "et",
            "en",
            "sur",
            "a",
            "ce",
            "d'où",
        ],
    ),
];

/// A guess at a question's tier-1 language, for a question no rule read and no `--lang` named:
/// Cyrillic is Russian; otherwise the Latin-script language whose common short words the
/// question uses most, with a letter only one of them writes (`ß`, `ñ`, `¿`, `ç`, …) counted
/// twice. `None` on a tie or no evidence. A guess, never a reading: `ask --misses` shows it
/// marked as one, and nothing classifies by it.
pub fn guess_lang(question: &str) -> Option<&'static str> {
    if question
        .chars()
        .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
    {
        return Some("ru");
    }
    let folded = fold(question).text;
    let mut scores = [0usize; 4];
    for word in folded.split(|c: char| c.is_whitespace() || c == '?' || c == '!' || c == '.') {
        let word = word.trim_matches(|c: char| c == '¿' || c == '¡' || c == '"');
        for (i, (_, words)) in FUNCTION_WORDS.iter().enumerate() {
            scores[i] += usize::from(words.contains(&word));
        }
    }
    for c in question.chars() {
        let only = match crate::fold::lower(c) {
            'ß' | 'ä' | 'ö' | 'ü' => Some(1),
            'ñ' | '¿' | '¡' | 'á' | 'í' | 'ó' | 'ú' => Some(2),
            'ç' | 'è' | 'ê' | 'à' | 'â' | 'î' | 'ô' | 'û' | 'ë' | 'ï' | 'œ' => Some(3),
            _ => None,
        };
        if let Some(i) = only {
            scores[i] += 2;
        }
    }
    let best = *scores.iter().max().expect("four languages");
    if best == 0 || scores.iter().filter(|&&s| s == best).count() > 1 {
        return None;
    }
    let i = scores
        .iter()
        .position(|&s| s == best)
        .expect("the max is there");
    Some(FUNCTION_WORDS[i].0)
}

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
    let best = best.or_else(|| {
        last_resort(f, lang, guess_lang(question)).map(|(rule, start, end)| (rule, 0, start, end))
    });
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

/// The first rule of [`LAST_RESORT`] that reads the folded question, and its topic's bounds.
/// Rule 28: not a rule of a language other than the one [`guess_lang`] puts the question in —
/// German `Was ergab sich …` is not English `was `.
fn last_resort(
    f: &str,
    lang: Option<&str>,
    guess: Option<&str>,
) -> Option<(&'static Rule, usize, usize)> {
    let first = f.split_whitespace().next().unwrap_or_default();
    LAST_RESORT
        .iter()
        .filter(|rule| lang.is_none_or(|l| l == rule.lang))
        .filter(|rule| guess.is_none_or(|g| g == rule.lang))
        .find_map(|rule| {
            if rule.infix.is_empty() {
                return (f.starts_with(rule.prefix) && f.len() > rule.prefix.len()).then_some((
                    rule,
                    rule.prefix.len(),
                    f.len(),
                ));
            }
            let asks = QUESTION_WORDS
                .iter()
                .filter(|(l, _)| *l == rule.lang)
                .flat_map(|(_, w)| w.iter())
                .any(|w| first == *w || (w.ends_with('\'') && first.starts_with(w)));
            let marked = f.split_whitespace().any(|w| {
                if rule.infix.starts_with('-') {
                    w.len() > rule.infix.len() && w.ends_with(rule.infix)
                } else {
                    w == rule.infix
                }
            });
            (marked && !asks).then_some((rule, 0, f.len()))
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
    fn a_yes_or_no_question_is_verification_only_when_nothing_else_reads_it() {
        for (q, intent) in [
            ("Is scurvy cured by citrus fruit?", Intent::Verify),
            ("Le scorbut est-il guéri par les agrumes ?", Intent::Verify),
            ("Лечится ли цинга цитрусовыми?", Intent::Verify),
            ("Why is cholera spread by water?", Intent::Explain),
            ("Where is smallpox mentioned?", Intent::Locate),
            (
                "Pourquoi le choléra est-il transmis par l'eau ?",
                Intent::Explain,
            ),
        ] {
            assert_eq!(classify(q, None).unwrap().intent, intent, "{q}");
        }
        // An inversion after a question word asks something other than whether.
        for q in ["Qu'est-il résulté de l'essai ?", "Сколько ли их было?"] {
            assert!(
                !classify(q, None).is_ok_and(|c| c.intent == Intent::Verify),
                "{q}"
            );
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
