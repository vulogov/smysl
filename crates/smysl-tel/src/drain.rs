//! Drain (He et al., ICWS 2017, as Drain3 runs it): log templates over event order (RFC
//! SMYSL-3 §9.7, §21, TL-P9).
//!
//! Our own implementation, because a template is evidence and has to be a function of the
//! records and the recipe alone. Drain is order-dependent: a message joins the most similar
//! cluster seen so far, and a cluster's template only ever generalises. So the order is fixed
//! as event order (§19.6), and the run is a function of the event set.
//!
//! **The recipe** ([`RECIPE`]) records every parameter:
//!
//! - **masks**, applied to each whitespace token before parsing, in order: a token that is
//!   wholly a UUID, an IPv4 address (with an optional port), a run of 8 or more hex digits (with
//!   an optional `0x`), or a decimal number becomes `<UUID>`, `<IP>`, `<HEX>` or `<NUM>`. Whole
//!   tokens, not substrings, so `id=42` is a word and `42` a number;
//! - **depth 4**: the tree's root, a layer by token count, one layer by the first token, and the
//!   leaves (Drain3 counts depth so), a token with a digit in it routed as `<*>`;
//! - **100 children** a node at most, the rest routed as `<*>`;
//! - **similarity 0.4**: a message joins the cluster at its leaf with the most positions equal
//!   to its template (a template's `<*>` equals nothing), if that is at least 0.4 of its length,
//!   in exact integer arithmetic; ties go to the cluster made first.
//!
//! A joined cluster's template becomes `<*>` where the message differs. Clusters whose final
//! templates are equal are one template, their counts summed.

use std::collections::BTreeMap;

/// The one recipe this build runs.
pub const RECIPE: &str = "tel/drain/1?depth=4&sim=0.4&children=100&masks=uuid,ip,hex,num";

const DEPTH: usize = 4;
const SIM_NUM: usize = 4;
const SIM_DEN: usize = 10;
const MAX_CHILDREN: usize = 100;
const WILD: &str = "<*>";

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_uuid(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && is_hex(p))
}

fn is_ipv4(t: &str) -> bool {
    let (addr, port) = match t.split_once(':') {
        Some((a, p)) => (a, Some(p)),
        None => (t, None),
    };
    if port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let octets: Vec<&str> = addr.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|o| {
            !o.is_empty()
                && o.len() <= 3
                && o.bytes().all(|b| b.is_ascii_digit())
                && o.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn is_hex_run(t: &str) -> bool {
    let h = t.strip_prefix("0x").unwrap_or(t);
    h.len() >= 8 && is_hex(h)
}

fn is_number(t: &str) -> bool {
    let t = t.strip_prefix('-').unwrap_or(t);
    let (int, frac) = match t.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (t, None),
    };
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// One token after masking.
pub fn mask(t: &str) -> &str {
    if is_uuid(t) {
        "<UUID>"
    } else if is_ipv4(t) {
        "<IP>"
    } else if is_hex_run(t) {
        "<HEX>"
    } else if is_number(t) {
        "<NUM>"
    } else {
        t
    }
}

/// A message's tokens, masked.
pub fn tokens(message: &str) -> Vec<String> {
    message
        .split_ascii_whitespace()
        .map(|t| mask(t).to_string())
        .collect()
}

#[derive(Default)]
struct Node {
    /// Children by routing token, and the order they were made in (for the children bound).
    children: BTreeMap<String, Node>,
    clusters: Vec<usize>,
}

struct Cluster {
    template: Vec<String>,
    count: u64,
}

/// A Drain run.
#[derive(Default)]
pub struct Drain {
    by_len: BTreeMap<usize, Node>,
    clusters: Vec<Cluster>,
}

impl Drain {
    pub fn new() -> Drain {
        Drain::default()
    }

    /// Add one message.
    pub fn add(&mut self, message: &str) {
        let toks = tokens(message);
        let mut node = self.by_len.entry(toks.len()).or_default();
        // Root, length and leaf are three of the depth's layers; the rest route by token.
        for t in toks.iter().take(DEPTH - 3) {
            let key = if t.bytes().any(|b| b.is_ascii_digit()) {
                WILD.to_string()
            } else {
                t.clone()
            };
            let key = if node.children.contains_key(&key) || node.children.len() + 1 < MAX_CHILDREN
            {
                key
            } else {
                WILD.to_string()
            };
            node = node.children.entry(key).or_default();
        }
        let mut best: Option<(usize, usize)> = None;
        for &c in &node.clusters {
            let sim = self.clusters[c]
                .template
                .iter()
                .zip(&toks)
                .filter(|(a, b)| a.as_str() != WILD && a == b)
                .count();
            if best.is_none_or(|(_, s)| sim > s) {
                best = Some((c, sim));
            }
        }
        match best {
            Some((c, sim)) if sim * SIM_DEN >= SIM_NUM * toks.len() => {
                let cl = &mut self.clusters[c];
                for (a, b) in cl.template.iter_mut().zip(&toks) {
                    if a != b {
                        *a = WILD.to_string();
                    }
                }
                cl.count += 1;
            }
            _ => {
                node.clusters.push(self.clusters.len());
                self.clusters.push(Cluster {
                    template: toks,
                    count: 1,
                });
            }
        }
    }

    /// The templates and their counts, by template text.
    pub fn templates(&self) -> BTreeMap<String, u64> {
        let mut out = BTreeMap::new();
        for c in &self.clusters {
            *out.entry(c.template.join(" ")).or_insert(0) += c.count;
        }
        out
    }
}

/// Drain over `messages`, in the order given.
pub fn run<'a>(messages: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, u64> {
    let mut d = Drain::new();
    for m in messages {
        d.add(m);
    }
    d.templates()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_take_whole_tokens_in_order() {
        assert_eq!(mask("123e4567-e89b-12d3-a456-426614174000"), "<UUID>");
        assert_eq!(mask("10.0.0.7"), "<IP>");
        assert_eq!(mask("10.0.0.7:8080"), "<IP>");
        assert_eq!(mask("10.0.0.256"), "10.0.0.256");
        assert_eq!(mask("0xdeadbeef"), "<HEX>");
        assert_eq!(mask("deadbeef00"), "<HEX>");
        assert_eq!(
            mask("12345678"),
            "<HEX>",
            "eight digits are a hex run first"
        );
        assert_eq!(mask("41"), "<NUM>");
        assert_eq!(mask("-0.5"), "<NUM>");
        assert_eq!(mask("id=42"), "id=42");
        assert_eq!(mask("accepted"), "accepted");
    }

    #[test]
    fn messages_of_one_shape_are_one_template_and_others_are_not() {
        let msgs = [
            "connection from 10.0.0.7 closed after 41 ms",
            "connection from 10.0.0.9 closed after 7 ms",
            "user alice logged in",
            "user bob logged in",
            "connection from 10.0.0.7 reset by peer",
            "disk sda full",
        ];
        let t = run(msgs);
        assert_eq!(
            t.get("connection from <IP> closed after <NUM> ms"),
            Some(&2)
        );
        assert_eq!(t.get("user <*> logged in"), Some(&2));
        assert_eq!(t.get("disk sda full"), Some(&1));
        assert_eq!(t.values().sum::<u64>(), 6);
        // Event order is the input; the same input gives the same templates.
        assert_eq!(run(msgs), t);
    }

    #[test]
    fn similarity_below_the_threshold_makes_a_new_cluster() {
        // 1 of 5 positions equal: 0.2 < 0.4.
        let t = run(["a b c d e", "a x y z w"]);
        assert_eq!(t.len(), 2, "{t:?}");
        // 2 of 5: exactly 0.4 joins.
        let t = run(["a b c d e", "a b y z w"]);
        assert_eq!(t.keys().collect::<Vec<_>>(), vec!["a b <*> <*> <*>"]);
        assert_eq!(
            run(["", ""]).get(""),
            Some(&2),
            "empty messages are one template"
        );
    }
}
