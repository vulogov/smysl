//! The pivot pass: one working language for a corpus that is not in one (F-6, §4.3.4).
//!
//! `--lang-policy pivot:<lang>` does **not** extract in another language. Extraction runs under
//! `source` exactly as it always does — the quote is the passage's own, in the passage's own
//! words, and a translated quote could not be `Present` against the text it came from — and then
//! this pass takes the units it produced and writes a second unit per unit whose `gist` is in the
//! named language, carrying the source unit in `deps`.
//!
//! That shape is SMYSL-2.4 §4.3.4's ("a second pass over `source`-policy units of the scope, one
//! unit per source unit, linked by `x.text/translates`, with the source unit in `deps` so rule M
//! caps it") and every part of it is load-bearing:
//!
//! * **`deps`, not a bare relation.** A translation is not an independent claim: it says what
//!   another unit says. With the source in `deps`, rule M caps its status at the source's, so a
//!   translation can never be better evidenced than the thing it translates. The
//!   `x.text/translates` edge is what a reader *queries*; `deps` is what the rules read.
//! * **One unit per source unit, written here and not by the model.** The model is asked for a
//!   sentence and nothing else. The span, the status, the source, the payload — the place, in
//!   other words — are copied from the unit being translated, so a pivot pass cannot invent a
//!   place, a claim or a relation. The worst a bad answer can do is produce a wrong sentence or
//!   no sentence at all, and the second is reported (`SMY-W453`).
//! * **`lang` on the core.** FC-1 put the language of the text fields inside the uid, which is
//!   what makes a pivot unit a different unit rather than a competing version of the same one.
//!   It is also why `body` and `detail` do **not** come along: one `lang` covers every text field
//!   of a unit, and a unit with an English gist and a Russian body would be lying about one of
//!   them. The pivot unit is an L0 statement whose L1 is one `deps` hop away.
//! * **Entities are left alone.** A-2.4 rule 7 makes an entity's identity its name, its language
//!   and its expression, so translating `Иерусалим` would not add a language to an entity — it
//!   would mint a second entity. Saying those two are one thing is `x.text/same-as`, which is
//!   TX-P7's engine and its judgement to make, not a side effect of a translation pass.
//!
//! Everything here is a function over data. The call belongs to [`run`](crate::text::run), like
//! every other call this crate makes.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    canonical_uid, lang::Script, lang::Scripts, Code, Diagnostic, LangTag, RelKind, Relation,
    UnitCore, UnitCoreBuilder,
};

/// What the pass produced.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Pivoted {
    /// One unit per source unit the answer covered, in the order the sources were given.
    pub units: Vec<UnitCore>,
    /// One `x.text/translates` edge per unit, from the translation to its source.
    pub relations: Vec<Relation>,
    /// Source units the answer did not cover (`SMY-W453`), as uids.
    pub missing: BTreeSet<smysl_core::Uid>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Which units a pivot pass is about: the claims, never the entities.
///
/// Returned as indices into the batch rather than clones, because the caller needs to put the
/// translations beside the originals and `UnitCore` is not small.
pub fn translatable(batch: &[UnitCore]) -> Vec<usize> {
    batch
        .iter()
        .enumerate()
        .filter(|(_, u)| u.schema.as_str() != smysl_core::schemas::text::ENTITY)
        .map(|(i, _)| i)
        .collect()
}

/// The numbered gists, one per line, as the prompt's input.
///
/// Numbered from 1 and by position, not by uid. A uid is 52 characters of base32 that a model
/// has no use for and every opportunity to mistype, and the pass does not need the model to
/// identify a unit — it needs it to keep an order. A newline inside a gist would break the one
/// line per unit rule the answer is parsed under, so it is folded to a space here; `L0` is one
/// sentence, so this is a defence rather than a transformation.
pub fn render(sources: &[&UnitCore]) -> String {
    sources
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let gist = u.gist.replace(['\n', '\r', '\t'], " ");
            format!("{}\t{}", i + 1, gist)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read the answer: a map from the position given to the sentence that came back.
///
/// Tolerant of the four ways a model numbers a list — `1<TAB>`, `1. `, `1) `, `1: ` — and of
/// blank lines, and of nothing else. A line whose number was not asked about is dropped and a
/// number that comes back twice keeps the first, because the alternative to both is a
/// translation attached to a unit it is not a translation of.
///
/// Lines are counted and not trusted: the caller compares the keys against the units it sent.
pub fn parse_answer(answer: &str, sent: usize) -> BTreeMap<usize, String> {
    let mut out: BTreeMap<usize, String> = BTreeMap::new();
    for line in crate::prompt::strip_echo(answer).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        let rest = &line[digits.len()..];
        let text = match rest.strip_prefix('\t') {
            Some(t) => t,
            None => match rest
                .strip_prefix(". ")
                .or_else(|| rest.strip_prefix(") "))
                .or_else(|| rest.strip_prefix(": "))
            {
                Some(t) => t,
                None => continue,
            },
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(n) = digits.parse::<usize>() else {
            continue;
        };
        if n == 0 || n > sent {
            continue;
        }
        out.entry(n).or_insert_with(|| text.to_string());
    }
    out
}

/// Build the pivot units from the sources and the answer.
///
/// `lang` is the language asked for. Each translation is held to its script where this build can
/// name one (`SMY-W436`, [`script_of_lang`](smysl_core::lang::script_of_lang)) — a model that
/// answers a `pivot:ru` request in English is the measured failure F-6 exists for, and the unit
/// is still written: it is a translation that claims to be Russian and is not, which a reader
/// should see rather than be spared.
pub fn units(sources: &[&UnitCore], answer: &BTreeMap<usize, String>, lang: &str) -> Pivoted {
    let mut out = Pivoted::default();
    let Ok(tag) = LangTag::new(lang) else {
        out.diagnostics
            .push(Diagnostic::new(Code::W453).with_message(format!(
                "`{lang}` is not a language tag; nothing was translated"
            )));
        return out;
    };
    let expected = smysl_core::lang::script_of_lang(lang);

    // Reported at the end rather than here, so the diagnostics come out in uid order whatever
    // order the answer's lines did — and separately from the units that *did* get a line and
    // could not be built, because those carry a reason of their own and must not be told about
    // twice.
    let mut no_line: Vec<smysl_core::Uid> = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let uid = canonical_uid(source);
        let Some(gist) = answer.get(&(i + 1)) else {
            out.missing.insert(uid);
            no_line.push(uid);
            continue;
        };
        let mut b = UnitCoreBuilder::new(source.schema.clone(), gist.as_str(), source.status);
        let mut deps: BTreeSet<_> = source.deps.clone();
        deps.insert(uid);
        b = b
            .deps(deps)
            .grounds(source.grounds.clone())
            .lang(tag.clone());
        if let Some(s) = source.source.clone() {
            b = b.source(s);
        }
        if let Some(p) = source.payload.clone() {
            b = b.payload(p);
        }
        // Preserved for the same reason rule X preserves it anywhere: a key this build does not
        // know is still part of what the source unit says, and dropping it here would make the
        // translation a claim about less than it translated.
        b.extra = source.extra.clone();

        let unit = match b.build() {
            Ok(u) => u,
            Err(e) => {
                out.diagnostics.push(
                    Diagnostic::on(Code::W453, uid)
                        .with_message(format!("the translation could not be built: {e}"))
                        .with_suggestion("the unit is staged in the source language only"),
                );
                out.missing.insert(uid);
                continue;
            }
        };

        if let Some(expected) = expected {
            if let Some(d) = off_script(&unit, expected, &tag) {
                out.diagnostics.push(d);
            }
        }

        out.relations.push(Relation::new(
            RelKind::Extension(smysl_core::schemas::text::TRANSLATES.to_string()),
            canonical_uid(&unit),
            uid,
        ));
        out.units.push(unit);
    }

    no_line.sort();
    for uid in no_line {
        out.diagnostics.push(
            Diagnostic::on(Code::W453, uid)
                .with_message(format!(
                    "the pivot pass returned no `{lang}` line for this unit"
                ))
                .with_suggestion("it is staged in the source language only; a rerun asks again"),
        );
    }
    out
}

/// A translation that is not in the script the language is written in (`SMY-W436`).
///
/// The same test `smysl_core::lang::verify` applies to an extraction, with the comparand
/// changed: there it is the passage's dominant script, here it is the script of the language
/// that was *asked for*. Short gists are exempt by [`MIN_LETTERS`](smysl_core::lang::MIN_LETTERS)
/// and a loanword is exempt by [`ABSENT_PERCENT`](smysl_core::lang::ABSENT_PERCENT), both for the
/// reasons stated where they are defined.
fn off_script(unit: &UnitCore, expected: Script, tag: &LangTag) -> Option<Diagnostic> {
    let s = Scripts::of(&unit.gist);
    if s.letters() < smysl_core::lang::MIN_LETTERS
        || s.percent(expected) >= smysl_core::lang::ABSENT_PERCENT
    {
        return None;
    }
    let found = match s.dominant() {
        Some(d) => d.to_string(),
        None => "no single script".to_string(),
    };
    Some(
        Diagnostic::on(Code::W436, canonical_uid(unit))
            .with_message(format!(
                "`pivot:{tag}` asked for {expected} and the gist is {found} ({}% {expected})",
                s.percent(expected)
            ))
            .with_suggestion(
                "re-run the pivot pass with a model that writes the language asked for",
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status};

    fn claim(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .expect("a gist and a status are all a claim needs")
    }

    fn entity(name: &str) -> UnitCore {
        let schema = smysl_core::SchemaId::parse(smysl_core::schemas::text::ENTITY)
            .expect("the entity type is a well-formed schema id");
        UnitCoreBuilder::new(schema, name, Status::Speculative)
            .build()
            .expect("an entity is a gist and a status too")
    }

    #[test]
    fn the_pass_is_about_claims_and_leaves_entities_to_tx_p7() {
        let batch = vec![
            entity("Иерусалим"),
            claim("Город был осаждён."),
            entity("Рим"),
        ];
        assert_eq!(translatable(&batch), vec![1]);
    }

    #[test]
    fn the_input_is_numbered_by_position_and_holds_one_line_per_unit() {
        let a = claim("Первое утверждение.");
        let b = claim("Второе\nутверждение.");
        let rendered = render(&[&a, &b]);
        assert_eq!(rendered, "1\tПервое утверждение.\n2\tВторое утверждение.");
    }

    /// The four numberings a model actually uses, and the three answers it must not be believed
    /// about: a number nobody asked for, a repeat, and a line with no number at all.
    #[test]
    fn the_answer_is_read_by_number_and_a_number_nobody_asked_about_is_dropped() {
        let answer = "1\tThe first claim.\n2. The second claim.\n3) The third.\n4: The fourth.\n\
                      9\tA claim about nothing.\n1\tA second first claim.\nloose commentary";
        let got = parse_answer(answer, 4);
        assert_eq!(got.len(), 4);
        assert_eq!(got[&1], "The first claim.");
        assert_eq!(got[&2], "The second claim.");
        assert_eq!(got[&3], "The third.");
        assert_eq!(got[&4], "The fourth.");
    }

    /// The shape §4.3.4 specifies: the source in `deps`, the edge, and the language on the core.
    #[test]
    fn a_translation_carries_its_source_in_deps_and_an_edge_back_to_it() {
        let source = claim("Город был осаждён зимой.");
        let uid = canonical_uid(&source);
        let answer = BTreeMap::from([(1, "The city was besieged in winter.".to_string())]);
        let p = units(&[&source], &answer, "en");

        assert_eq!(p.units.len(), 1);
        let u = &p.units[0];
        assert_eq!(u.gist, "The city was besieged in winter.");
        assert_eq!(u.lang.as_ref().map(|l| l.as_str()), Some("en"));
        assert!(u.deps.contains(&uid), "the source is in deps (rule M)");
        assert_ne!(
            canonical_uid(u),
            uid,
            "a different language is a different unit"
        );

        assert_eq!(p.relations.len(), 1);
        let r = &p.relations[0];
        assert_eq!(
            r.kind,
            RelKind::Extension(smysl_core::schemas::text::TRANSLATES.to_string())
        );
        assert_eq!(r.from, canonical_uid(u));
        assert_eq!(r.to, uid);
        assert!(p.missing.is_empty());
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    }

    /// The place is the source unit's, which is the whole reason the model is not asked for one.
    #[test]
    fn the_translation_keeps_the_place_and_not_the_prose_around_it() {
        let source =
            UnitCoreBuilder::new(KernelType::Claim, "Город был осаждён.", Status::Speculative)
                .body("Длинное изложение того же.")
                .payload(vec![0xa0])
                .build()
                .expect("a claim with a body and a payload");
        let answer = BTreeMap::from([(1, "The city was besieged.".to_string())]);
        let p = units(&[&source], &answer, "en");
        let u = &p.units[0];
        assert_eq!(u.payload.as_deref(), Some([0xa0].as_slice()));
        assert_eq!(
            u.body, None,
            "one `lang` covers every text field, so the untranslated body is not carried"
        );
    }

    /// `SMY-W453`: a unit the answer skipped is reported rather than counted as translated.
    #[test]
    fn a_unit_the_answer_skipped_is_reported_and_left_in_its_own_language() {
        let a = claim("Первое.");
        let b = claim("Второе.");
        let answer = BTreeMap::from([(1, "The first.".to_string())]);
        let p = units(&[&a, &b], &answer, "en");
        assert_eq!(p.units.len(), 1);
        assert_eq!(p.missing.len(), 1);
        assert!(p.missing.contains(&canonical_uid(&b)));
        assert_eq!(
            p.diagnostics
                .iter()
                .filter(|d| d.code == Code::W453)
                .count(),
            1
        );
    }

    /// `SMY-W436`, with the comparand the policy named rather than the passage's.
    #[test]
    fn a_translation_in_the_wrong_script_is_reported_and_still_written() {
        let source = claim("The city was besieged in winter.");
        let answer = BTreeMap::from([(1, "The city was besieged in winter.".to_string())]);
        let p = units(&[&source], &answer, "ru");
        assert_eq!(p.units.len(), 1, "the unit is written, not withheld");
        let d = p
            .diagnostics
            .iter()
            .find(|d| d.code == Code::W436)
            .expect("the script check fired");
        assert!(d.message.contains("pivot:ru"), "{:?}", d.message);
    }

    /// A language whose script this build cannot name is unevaluable, and silence is the answer.
    #[test]
    fn a_language_with_no_script_this_build_names_is_not_reported_either_way() {
        let source = claim("The city was besieged in winter.");
        let answer = BTreeMap::from([(1, "Mji ulizingirwa wakati wa baridi.".to_string())]);
        let p = units(&[&source], &answer, "sw");
        assert_eq!(p.units.len(), 1);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    }
}
