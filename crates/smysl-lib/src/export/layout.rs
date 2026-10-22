//! Layout keys, and the view-id segments made from them (SMYSL-2.7 §3.4).

use smysl_core::ids::base32;
use smysl_core::{Uid, ViewId};
use smysl_graph::{closure, EdgeSet, Store};
use std::collections::{BTreeMap, BTreeSet};

/// The longest slug before a hash suffix, in bytes.
const SLUG_MAX: usize = 48;

/// How many base32 characters of BLAKE3(key) a suffixed slug carries.
const SUFFIX_CHARS: usize = 10;

/// The two file names a slug must never take: the index and the file of unknown records.
const RESERVED: [&str; 2] = ["unattributed", "export-index"];

/// A layout key as a view-id segment, and whether it is the key unchanged.
///
/// Lowercase ASCII, every byte outside `[a-z0-9_-]` mapped to `-` and runs of `-` collapsed.
/// A result that is empty or does not start with a letter gets a `k` in front, and the whole
/// is cut to 48 bytes. `export/` followed by what comes back is always a [`ViewId`].
///
/// The flag is false when the mapping changed anything. A key that is not already a segment
/// cannot be read back from its slug, so the caller gives it a hash suffix (`SMY-W497`), as it
/// does when two keys of one export map to one slug.
pub fn slug(key: &str) -> (String, bool) {
    let mut s = String::with_capacity(key.len().min(SLUG_MAX));
    for c in key.bytes().map(|b| b.to_ascii_lowercase()) {
        let c = match c {
            b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' => c,
            _ => b'-',
        };
        if c == b'-' && s.ends_with('-') {
            continue;
        }
        s.push(c as char);
    }
    if !s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s.insert(0, 'k');
    }
    s.truncate(SLUG_MAX);
    let same = s == key;
    (s, same)
}

/// The slug with its hash suffix.
fn suffixed(slug: &str, key: &str) -> String {
    let h = *blake3::hash(key.as_bytes()).as_bytes();
    format!("{slug}-{}", base32(&h, SUFFIX_CHARS).to_ascii_lowercase())
}

/// The slug of every key, suffixed where it must be, and how many were.
///
/// A key's slug is suffixed when [`slug`] changed it, when another key of the same export maps
/// to the same slug, or when it would name a reserved file. Each is decided from the keys
/// alone, so the names are a function of the partition.
pub(crate) fn slugs(keys: &BTreeSet<String>) -> (BTreeMap<String, String>, usize) {
    let plain: BTreeMap<&String, (String, bool)> = keys.iter().map(|k| (k, slug(k))).collect();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (s, _) in plain.values() {
        *seen.entry(s.as_str()).or_default() += 1;
    }
    let mut suffixes = 0;
    let out = plain
        .iter()
        .map(|(k, (s, same))| {
            let clash = seen[s.as_str()] > 1 || RESERVED.contains(&s.as_str());
            if *same && !clash {
                ((*k).clone(), s.clone())
            } else {
                suffixes += 1;
                ((*k).clone(), suffixed(s, k))
            }
        })
        .collect();
    (out, suffixes)
}

/// The view id a slug's file carries.
pub(crate) fn view_id(slug: &str) -> ViewId {
    ViewId::new(format!("export/{slug}")).expect("a slug is a view-id segment")
}

/// The key a unit takes under `by-view`: the least corpus view whose membership holds it.
///
/// Membership is the view's roots closed along `deps` and `grounds`, as everywhere else. A unit
/// in several views goes to the least view id, in byte order, so the partition is a function of
/// the store. One in none goes to `unkeyed`. Views under `export/` are an earlier export's own
/// and are never a key: re-exporting an export must not file units by the files they came in.
///
/// Also returns, per key, the view it names, which travels in that key's file.
pub(crate) fn by_view(
    store: &Store,
    sel: &BTreeSet<Uid>,
) -> (BTreeMap<Uid, String>, BTreeMap<String, ViewId>) {
    let g = store.adjacency();
    let mut owner: BTreeMap<Uid, String> = BTreeMap::new();
    let mut keyed: BTreeMap<String, ViewId> = BTreeMap::new();
    let mut views: Vec<_> = store
        .views()
        .filter(|v| !v.id.as_str().starts_with("export/"))
        .collect();
    views.sort_by(|a, b| a.id.cmp(&b.id));
    for v in views {
        let key = v.id.as_str().replace('/', "-");
        let roots: Vec<_> = v.roots.iter().filter_map(|u| g.id(u)).collect();
        for n in closure(g, &roots, &EdgeSet::support()) {
            if let Some(u) = g.uid(n).filter(|u| sel.contains(u)) {
                if !owner.contains_key(u) {
                    owner.insert(*u, key.clone());
                    keyed.entry(key.clone()).or_insert_with(|| v.id.clone());
                }
            }
        }
    }
    let units = sel
        .iter()
        .map(|u| (*u, owner.remove(u).unwrap_or_else(|| "unkeyed".to_string())))
        .collect();
    (units, keyed)
}

/// The key a unit takes under `by-expression` or `by-collection` (§3.4): the alias of the
/// manifest its source names, or that alias's collection, which is its namespace up to the
/// first `/` (`kjv` for `kjv/08-rut`, as `collection:kjv` reads it, SMYSL-2.4 scope). A unit
/// whose source names no manifest the store holds goes to `unkeyed`.
///
/// An alias's `/` and `:` become `-`, as a view id's `/` does under `by-view`, so the common
/// alias is a plain segment and needs no hash suffix. Two aliases that would meet on one key
/// that way keep their own spelling instead, which [`slugs`] then suffixes apart: two
/// expressions are never filed together because their names differ only in punctuation.
pub(crate) fn by_manifest(
    store: &Store,
    sel: &BTreeSet<Uid>,
    collection: bool,
) -> BTreeMap<Uid, String> {
    let alias_of = |u: &Uid| -> Option<String> {
        let mid = store.get(u)?.core.source.as_ref()?.manifest()?;
        let alias = store.manifest(mid)?.alias.as_str();
        Some(match (collection, alias.split_once('/')) {
            (true, Some((head, _))) => head.to_string(),
            _ => alias.to_string(),
        })
    };
    let raw: BTreeMap<Uid, Option<String>> = sel.iter().map(|u| (*u, alias_of(u))).collect();
    let plain = |a: &str| a.replace(['/', ':'], "-");
    let mut meets: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for a in raw.values().flatten() {
        meets.entry(plain(a)).or_default().insert(a);
    }
    raw.iter()
        .map(|(u, a)| {
            let key = match a {
                None => "unkeyed".to_string(),
                Some(a) if meets[&plain(a)].len() > 1 => a.clone(),
                Some(a) => plain(a),
            };
            (*u, key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_always_a_view_id_segment() {
        for key in [
            "",
            "all",
            "v-doc",
            "KJV",
            "1611",
            "a b/c",
            "--x--",
            "é",
            "unattributed",
            "export-index",
            &"z".repeat(80),
        ] {
            let (s, _) = slug(key);
            assert!(
                ViewId::new(format!("export/{s}")).is_ok(),
                "{key:?} gave {s:?}"
            );
            assert!(s.len() <= SLUG_MAX);
        }
    }

    #[test]
    fn slugs_are_the_table_of_section_3_4() {
        assert_eq!(slug("all"), ("all".into(), true));
        assert_eq!(slug("v-doc"), ("v-doc".into(), true));
        assert_eq!(slug("KJV"), ("kjv".into(), false));
        assert_eq!(slug("1611"), ("k1611".into(), false));
        assert_eq!(slug(""), ("k".into(), false));
        assert_eq!(slug("a b//c"), ("a-b-c".into(), false));
        assert_eq!(slug("a--b"), ("a-b".into(), false));
        assert_eq!(slug(&"z".repeat(80)).0.len(), SLUG_MAX);
    }

    #[test]
    fn changed_colliding_and_reserved_keys_are_suffixed() {
        let keys: BTreeSet<String> = ["all", "KJV", "kjv", "unattributed", "export-index"]
            .into_iter()
            .map(String::from)
            .collect();
        let (out, n) = slugs(&keys);
        assert_eq!(out["all"], "all");
        // `kjv` is its own slug, but `KJV` maps there too: both are suffixed, and apart.
        assert!(out["kjv"].starts_with("kjv-") && out["KJV"].starts_with("kjv-"));
        assert_ne!(out["kjv"], out["KJV"]);
        assert!(out["unattributed"].starts_with("unattributed-"));
        assert!(out["export-index"].starts_with("export-index-"));
        assert_eq!(n, 4);
        for s in out.values() {
            assert!(ViewId::new(format!("export/{s}")).is_ok(), "{s}");
            assert_eq!(slugs(&keys).0.values().filter(|t| *t == s).count(), 1);
        }
    }
}
