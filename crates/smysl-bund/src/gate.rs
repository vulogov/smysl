//! The word gate (RFC SMYSL-4 §6).
//!
//! It reads a script the way Bund2's parser does (`bund2-syntax/src/lib.rs` at 0.56.0) and
//! refuses it before anything is written or started. **It is not the boundary** (D-B2): a
//! script can build a word's name from strings at run time, and only the sandbox contains that.
//! The gate is here so that a mistake is named as one.
//!
//! It also keeps the bridge's own framing intact. The script is placed between a preamble and
//! an encoder in one file, so a string, a literal or a bracket the script leaves open would
//! swallow the encoder. The gate refuses all three.

use crate::error::BundError;
use crate::policy::{QueryPolicy, WordPolicy};
use crate::words;

/// One word a script names, as a call or as a pointer (`` `name ``).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub word: String,
    /// Byte offset in the script.
    pub at: usize,
    /// Written as a pointer, `` `word ``.
    pub pointer: bool,
    /// A bare word directly inside `[ … ]`. Bund2 reads it as a STRING and does not run it: it
    /// is a clause's functor or atom, `[ intake W S ]`, not a call. The gate does not check it.
    pub quoted: bool,
}

impl Named {
    /// Whether the word is run where it stands: neither quoted nor a pointer.
    pub fn is_call(&self) -> bool {
        !self.quoted && !self.pointer
    }
}

/// Read `script` and return the words it names, or refuse it.
pub fn check(script: &str, policy: &QueryPolicy) -> Result<Vec<Named>, BundError> {
    if script.len() > policy.max_script_bytes {
        return Err(BundError::ScriptTooLarge {
            bytes: script.len(),
            max: policy.max_script_bytes,
        });
    }
    let named = scan(script, policy.max_nesting_depth)?;
    for n in named.iter().filter(|n| !n.quoted) {
        if let Some((_, _, why)) = words::refused(&n.word) {
            return Err(violation(n, why));
        }
        match &policy.word_policy {
            WordPolicy::Allowlist(allow) if !allow.contains(&n.word) => {
                let why = if n.word == "smysl.open" || n.word == "smysl.open." {
                    "the bridge opens the library; the script reads it with `:lib var?`"
                } else {
                    "not on the allowlist"
                };
                return Err(violation(n, why));
            }
            WordPolicy::Denylist(deny) if deny.contains(&n.word) => {
                return Err(violation(n, "on the caller's denylist"));
            }
            _ => {}
        }
    }
    Ok(named)
}

fn violation(n: &Named, why: &str) -> BundError {
    BundError::PolicyViolation {
        reason: format!("{why} (at byte {})", n.at),
        token: Some(n.word.clone()),
    }
}

fn syntax(at: usize, what: &str) -> BundError {
    BundError::InvalidScriptOrArgs {
        code: String::new(),
        message: format!("{what} at byte {at}"),
    }
}

/// The words a script names, with strings, literals, atoms, numbers, stack names and comments
/// skipped as Bund2 skips them.
pub fn scan(script: &str, max_depth: usize) -> Result<Vec<Named>, BundError> {
    let chars: Vec<(usize, char)> = script.char_indices().collect();
    let mut out = Vec::new();
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).map_or(script.len(), |(o, _)| *o);
    let peek = |i: usize| chars.get(i).map(|(_, c)| *c);
    // A run of characters up to whitespace or a bracket.
    let run = |mut i: usize| {
        while let Some(c) = peek(i) {
            if c.is_whitespace() || "{}[]()".contains(c) {
                break;
            }
            i += 1;
        }
        i
    };
    while let Some(c) = peek(i) {
        let start = i;
        match c {
            c if c.is_whitespace() => i += 1,
            // `//` between tokens is a comment to the end of the line.
            '/' if peek(i + 1) == Some('/') => {
                while peek(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
            }
            '{' | '[' | '(' => {
                open.push((c, at(i)));
                if open.len() > max_depth {
                    return Err(syntax(at(i), &format!("nested deeper than {max_depth}")));
                }
                i += 1;
            }
            '}' | ']' | ')' => {
                let want = match c {
                    '}' => '{',
                    ']' => '[',
                    _ => '(',
                };
                match open.pop() {
                    Some((o, _)) if o == want => {}
                    _ => return Err(syntax(at(i), &format!("`{c}` closes nothing open"))),
                }
                i += 1;
            }
            // A string: a backslash keeps the next character, so `\"` does not end it.
            '"' => {
                i += 1;
                loop {
                    match peek(i) {
                        None => return Err(syntax(at(start), "unterminated string")),
                        Some('"') => break,
                        Some('\\') => i += 2,
                        Some(_) => i += 1,
                    }
                }
                i += 1;
            }
            // A literal: verbatim to the next `'`.
            '\'' => {
                i += 1;
                while peek(i).is_some_and(|c| c != '\'') {
                    i += 1;
                }
                if peek(i).is_none() {
                    return Err(syntax(at(start), "unterminated literal"));
                }
                i += 1;
            }
            // A pointer names a word as surely as a call does.
            '`' => {
                let end = run(i + 1);
                let word: String = chars[i + 1..end].iter().map(|(_, c)| c).collect();
                out.push(Named {
                    word,
                    at: at(start),
                    pointer: true,
                    quoted: false,
                });
                i = end;
            }
            // A stack name, an atom (a STRING), a command: none names a word. A number does not.
            '@' | ':' | ';' => i = run(i + 1),
            c if c.is_ascii_digit() => i = run(i),
            '+' | '-' if peek(i + 1).is_some_and(|d| d.is_ascii_digit()) => i = run(i),
            _ => {
                let end = run(i);
                let word: String = chars[i..end].iter().map(|(_, c)| c).collect();
                out.push(Named {
                    word,
                    at: at(start),
                    pointer: false,
                    quoted: open.last().is_some_and(|(o, _)| *o == '['),
                });
                i = end.max(i + 1);
            }
        }
    }
    if let Some((c, o)) = open.pop() {
        return Err(syntax(o, &format!("`{c}` is never closed")));
    }
    Ok(out)
}

/// The words that make a KB. Each is followed by `cap kb.inferences` (§9).
const KB_MAKERS: [&str; 3] = ["smysl.kb", "kb!", "kb.new"];

/// `script` with `max_inferences` applied to every KB it makes, and to every cap it sets: each
/// KB maker is followed by `<cap> kb.inferences`, and a cap the script sets above `cap` is
/// lowered to it. Only the words in `named` (from [`check`]) that run where they stand are
/// touched, and no line is added, so an error's line number is the script's.
///
/// It bounds what a script asks of a KB it made. A script can still edit a KB's map by hand,
/// so it is not the boundary; the wall clock is.
pub fn with_inference_cap(script: &str, named: &[Named], cap: u64) -> String {
    let mut out = String::with_capacity(script.len());
    let mut from = 0;
    for n in named.iter().filter(|n| n.is_call()) {
        let end = n.at + n.word.len();
        if KB_MAKERS.contains(&n.word.as_str()) {
            out.push_str(&script[from..end]);
            out.push_str(&format!(" {cap} kb.inferences"));
            from = end;
        } else if n.word == "kb.inferences" || n.word == "kb.inferences." {
            // `>` and `<` compare the top with the value under it: `n cap <` is `cap < n`.
            out.push_str(&script[from..n.at]);
            out.push_str(&format!("dup {cap} < {{ drop {cap} }} if "));
            out.push_str(&n.word);
            from = end;
        }
    }
    out.push_str(&script[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_of(s: &str) -> Vec<String> {
        scan(s, 32).unwrap().into_iter().map(|n| n.word).collect()
    }

    fn refused_token(script: &str) -> Option<String> {
        match check(script, &QueryPolicy::default()) {
            Err(BundError::PolicyViolation { token, .. }) => token,
            other => panic!("{script}: {other:?}"),
        }
    }

    #[test]
    fn strings_literals_atoms_and_comments_name_no_word() {
        assert_eq!(
            words_of(
                r#":lib var? "system.shell \" rm" 'sh -c' // fs.rm here
            :x 42 -3 3.5 @other smysl.get"#
            ),
            ["var?", "smysl.get"]
        );
    }

    #[test]
    fn a_pointer_names_its_word() {
        assert_eq!(words_of("`system.shell !"), ["system.shell", "!"]);
        assert_eq!(refused_token("`sh"), Some("sh".into()));
    }

    #[test]
    fn a_word_in_a_list_is_data_and_one_in_a_lambda_is_not() {
        let p = QueryPolicy::default();
        assert!(check(":k [ [ intake W S ] ] kb! var", &p).is_ok());
        assert_eq!(refused_token("[ { sh } ]"), Some("sh".into()));
        assert_eq!(refused_token("[ `sh ]"), Some("sh".into()));
        assert_eq!(refused_token("( sh )"), Some("sh".into()));
    }

    #[test]
    fn the_cap_follows_each_kb_and_lowers_each_cap() {
        let s =
            ":k [ [ kb.new ] ] kb! var\n:j kb.new 5000 kb.inferences var :lib var? \"q\" smysl.kb";
        let named = check(s, &QueryPolicy::default()).unwrap();
        assert_eq!(
            with_inference_cap(s, &named, 100),
            ":k [ [ kb.new ] ] kb! 100 kb.inferences var\n\
             :j kb.new 100 kb.inferences 5000 dup 100 < { drop 100 } if kb.inferences var \
             :lib var? \"q\" smysl.kb 100 kb.inferences"
        );
    }

    #[test]
    fn brackets_are_not_part_of_a_word() {
        assert_eq!(words_of("{ dup [ swap ] }"), ["dup", "swap"]);
    }

    #[test]
    fn the_refused_words_are_refused() {
        for s in [
            "\"ls\" system.shell",
            "sh",
            ":lib var? smysl.retract",
            "\"x\" !!",
            "\"system.\" \"shell\" + call, !",
            "\"40 2 +\" compile",
            ":w resolve",
            "time.now",
            "id.uuid",
            "\"http://x\" url",
            "fs.ls",
        ] {
            assert!(refused_token(s).is_some(), "{s}");
        }
    }

    #[test]
    fn smysl_open_is_the_preambles() {
        assert_eq!(
            refused_token("\"/etc\" smysl.open"),
            Some("smysl.open".into())
        );
    }

    #[test]
    fn a_read_only_query_passes() {
        let s = ":lib var? \"from library where kind = claim\" smysl.query :ranked get";
        assert!(check(s, &QueryPolicy::default()).is_ok());
    }

    #[test]
    fn an_unknown_word_is_off_the_allowlist() {
        assert_eq!(refused_token("frobnicate"), Some("frobnicate".into()));
    }

    #[test]
    fn under_a_denylist_only_the_refused_and_the_denied_are_refused() {
        let p = QueryPolicy {
            word_policy: WordPolicy::Denylist(["smysl.dossier".to_string()].into()),
            ..QueryPolicy::default()
        };
        assert!(check("frobnicate", &p).is_ok());
        assert!(check("sh", &p).is_err());
        assert!(check(":lib var? \"q\" smysl.dossier", &p).is_err());
    }

    /// What would let the script swallow the encoder after it.
    #[test]
    fn open_framing_is_refused() {
        for s in ["\"open", "'open", "{ dup", "[ 1 2", "dup }", "\"a\\\""] {
            assert!(
                matches!(
                    check(s, &QueryPolicy::default()),
                    Err(BundError::InvalidScriptOrArgs { .. })
                ),
                "{s}"
            );
        }
    }

    #[test]
    fn size_and_depth() {
        let p = QueryPolicy {
            max_script_bytes: 4,
            ..QueryPolicy::default()
        };
        assert!(matches!(
            check("dup dup", &p),
            Err(BundError::ScriptTooLarge { bytes: 7, max: 4 })
        ));
        let deep = format!("{}1{}", "[ ".repeat(33), " ]".repeat(33));
        assert!(matches!(
            check(&deep, &QueryPolicy::default()),
            Err(BundError::InvalidScriptOrArgs { .. })
        ));
    }
}
