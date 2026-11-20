//! JSON Pointers (RFC 6901) and the patterns profiles write over them (RFC SMYSL-3 §8.1).
//!
//! A [`Pointer`] names one leaf: `/spec/containers/web/image`, where `web` is a keyed array
//! element's key (§8.1 step 2). Its text is inside a change unit's uid, so it has one spelling:
//! `~` is written `~0`, `/` is written `~1`, and nothing else is escaped.
//!
//! A [`Pattern`] is a pointer in which a segment written `*` matches any one segment. A literal
//! key `*` is written `~2`. That is the one extension to RFC 6901, and it appears only in
//! profiles, never in units — so [`Pointer::parse`] refuses `~2`, and a pattern's `*` cannot be
//! confused with a key that happens to be `*`.

use core::fmt;

/// A JSON Pointer, as its unescaped segments. The empty pointer names the whole document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pointer(pub Vec<String>);

/// Why text is not a pointer or a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerError {
    pub text: String,
    pub why: &'static str,
}

impl fmt::Display for PointerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a pointer: {}", self.text, self.why)
    }
}

impl std::error::Error for PointerError {}

/// RFC 6901's escaping of one segment, in its one spelling.
pub fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Undo [`escape`], with `~2` admitted only when `star` is set (in a pattern).
fn unescape(text: &str, segment: &str, star: bool) -> Result<String, PointerError> {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            Some('2') if star => out.push('*'),
            Some('2') => {
                return Err(PointerError {
                    text: text.to_string(),
                    why: "`~2` is a profile pattern's, never a pointer's",
                })
            }
            _ => {
                return Err(PointerError {
                    text: text.to_string(),
                    why: "`~` must be followed by 0 or 1",
                })
            }
        }
    }
    Ok(out)
}

/// The segments of `text`, which is empty or begins with `/`.
fn split(text: &str) -> Result<Vec<&str>, PointerError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let rest = text.strip_prefix('/').ok_or_else(|| PointerError {
        text: text.to_string(),
        why: "a pointer begins with `/`",
    })?;
    Ok(rest.split('/').collect())
}

impl Pointer {
    /// The whole document.
    pub fn root() -> Pointer {
        Pointer(Vec::new())
    }

    pub fn parse(text: &str) -> Result<Pointer, PointerError> {
        split(text)?
            .into_iter()
            .map(|s| unescape(text, s, false))
            .collect::<Result<Vec<_>, _>>()
            .map(Pointer)
    }

    /// This pointer with one more segment.
    pub fn child(&self, segment: impl Into<String>) -> Pointer {
        let mut v = self.0.clone();
        v.push(segment.into());
        Pointer(v)
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.0 {
            write!(f, "/{}", escape(s))?;
        }
        Ok(())
    }
}

/// One segment of a pattern.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Seg {
    /// `*`: any one segment.
    Any,
    /// A segment matched exactly, unescaped.
    Lit(String),
}

/// A profile's pointer pattern (§8.1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pattern(pub Vec<Seg>);

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern, PointerError> {
        split(text)?
            .into_iter()
            .map(|s| {
                if s == "*" {
                    Ok(Seg::Any)
                } else {
                    unescape(text, s, true).map(Seg::Lit)
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Pattern)
    }

    /// The number of segments: §8.1 step 4 takes the **longest** class pattern that matches.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the pattern names exactly this pointer.
    pub fn matches(&self, p: &Pointer) -> bool {
        self.0.len() == p.0.len() && self.covers(p)
    }

    /// Whether the pattern matches this pointer or a prefix of it: a class declared on
    /// `/metadata/labels` covers every label (§8.1 step 4).
    pub fn covers(&self, p: &Pointer) -> bool {
        self.0.len() <= p.0.len()
            && self.0.iter().zip(&p.0).all(|(s, seg)| match s {
                Seg::Any => true,
                Seg::Lit(l) => l == seg,
            })
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.0 {
            match s {
                Seg::Any => f.write_str("/*")?,
                Seg::Lit(l) => write!(f, "/{}", escape(l).replace('*', "~2"))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str) -> Pointer {
        Pointer::parse(t).unwrap()
    }

    #[test]
    fn rfc_6901_escaping_has_one_spelling() {
        let ptr = p("/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration");
        assert_eq!(
            ptr.segments()[2],
            "kubectl.kubernetes.io/last-applied-configuration"
        );
        assert_eq!(
            ptr.to_string(),
            "/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration"
        );
        assert_eq!(p("/a~0b").segments(), ["a~b"]);
        assert_eq!(p("").to_string(), "");
        assert_eq!(p("/").segments(), [""], "one empty segment, not the root");
        assert!(Pointer::parse("a").is_err());
        assert!(Pointer::parse("/a~").is_err());
        assert!(Pointer::parse("/a~3").is_err());
        assert!(Pointer::parse("/~2").is_err(), "~2 is never a pointer's");
    }

    #[test]
    fn a_star_matches_one_segment_and_tilde_two_is_a_star() {
        let pat = Pattern::parse("/status/conditions/*/lastProbeTime").unwrap();
        assert!(pat.matches(&p("/status/conditions/Ready/lastProbeTime")));
        assert!(!pat.matches(&p("/status/conditions/Ready/x/lastProbeTime")));
        assert!(!pat.matches(&p("/status/conditions/lastProbeTime")));
        let star = Pattern::parse("/labels/~2").unwrap();
        assert!(star.matches(&p("/labels/*")));
        assert!(!star.matches(&p("/labels/app")));
        assert_eq!(star.to_string(), "/labels/~2");
        assert_eq!(pat.to_string(), "/status/conditions/*/lastProbeTime");
    }

    #[test]
    fn a_pattern_covers_what_is_under_it() {
        let labels = Pattern::parse("/metadata/labels").unwrap();
        assert!(labels.covers(&p("/metadata/labels/app")));
        assert!(labels.covers(&p("/metadata/labels")));
        assert!(!labels.matches(&p("/metadata/labels/app")));
        assert!(!labels.covers(&p("/metadata")));
    }
}
