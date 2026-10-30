//! What an export selects, as a query (SMYSL-2.7 §3.6, §3.9; TX-P11 step 8).
//!
//! `--scope` is an `sq/1` query (SMYSL-2.5). [`query`] runs it through whatever ports the caller
//! built, a library's or a plain store's, and the selection is the units it answers. `by-query`
//! files them by a `group by` field, [`group`], and that is computed over a plain store of the
//! snapshot, never over a library's ports. That is deliberate: a group value then depends on
//! the records alone, so `verify`'s P3, which re-exports from an in-memory store, cuts the same
//! groups, where a library's port could answer a field from its catalogue.
//!
//! [`import_scope`] is §3.9's resolution of `from import:<collection>`: for a collection written
//! by `lib export`, the union of the roots of its `export/` views. Every unit that arrived is
//! not the same set, since closure units would become selected and the partition and the roots
//! would move.

use crate::access::LibraryWrite;
use crate::export::{LibError, Selection};
use smysl_core::{Record, Uid};
use smysl_graph::Store;
use smysl_query::{Answer, Env, PlanOptions, StoreScope};
use std::collections::{BTreeMap, BTreeSet};

/// The query language version an index records beside a scope (key 3).
pub const SQ_VERSION: &str = "sq/1";

fn refuse(ds: Vec<smysl_core::Diagnostic>) -> LibError {
    LibError::Query(
        ds.iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// The units `text` answers over `env`, in uid order, with the text recorded as the scope.
///
/// A ranking's units are taken whatever their order, since an export is a set; a `group by`
/// answer's are taken from every group; a `count` names no units and is refused.
pub fn query(env: &Env<'_>, text: &str) -> Result<Selection, LibError> {
    let parsed = smysl_query::parse(text).map_err(|d| refuse(vec![d]))?;
    let opts = PlanOptions::default();
    let plan = smysl_query::plan(&parsed.query, env, &opts).map_err(refuse)?;
    let run = smysl_query::run(&plan, env, &opts).map_err(refuse)?;
    let units: BTreeSet<Uid> = match run.answer {
        Answer::Scope(u) | Answer::Packed(u) => u.into_iter().collect(),
        Answer::Ranked(f) => f.into_iter().map(|x| x.uid).collect(),
        Answer::Groups(g) => g.into_iter().flat_map(|(_, u)| u).collect(),
        Answer::Count(_) => {
            return Err(LibError::Query(
                "a `count` names no units to export".to_string(),
            ))
        }
        #[allow(unreachable_patterns)]
        _ => {
            return Err(LibError::Query(
                "an answer this build cannot export".to_string(),
            ))
        }
    };
    Ok(Selection::of(units).scoped(text))
}

/// `by-query`'s keys: the value of `field` for every selected unit, by `group by` over a plain
/// store of `store`. A unit `group by` puts in no group goes to `unkeyed`.
pub fn group(store: &Store, sel: Selection, field: &str) -> Result<Selection, LibError> {
    let scope = StoreScope::new("export", store);
    let env = Env::new(store, &scope);
    let text = format!("from store:export\ngroup by {field}");
    let parsed = smysl_query::parse(&text).map_err(|d| refuse(vec![d]))?;
    let opts = PlanOptions::default();
    let plan = smysl_query::plan(&parsed.query, &env, &opts).map_err(refuse)?;
    let run = smysl_query::run(&plan, &env, &opts).map_err(refuse)?;
    let Answer::Groups(groups) = run.answer else {
        return Err(LibError::Query(format!(
            "`group by {field}` answered no groups"
        )));
    };
    let mut keys: BTreeMap<Uid, String> = BTreeMap::new();
    for (value, units) in groups {
        for u in units {
            keys.entry(u).or_insert_with(|| value.clone());
        }
    }
    let keys = sel
        .units()
        .iter()
        .map(|u| (*u, keys.remove(u).unwrap_or_else(|| "unkeyed".to_string())))
        .collect();
    Ok(sel.grouped(field, keys))
}

/// §3.9's `from import:<name>` for a collection written by `lib export`: the union of the roots
/// of the `export/` views among what that import retained. `None` when it retained none, which
/// is an import of files no export wrote; the query then means what `sq` says it does.
pub fn import_scope(lib: &impl LibraryWrite, name: &str) -> Result<Option<Selection>, LibError> {
    let roots: BTreeSet<Uid> = crate::import::retained(lib, name)?
        .iter()
        .filter_map(|r| match r {
            Record::View(v) if v.id.as_str().starts_with("export/") => Some(v.roots.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    if roots.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        Selection::of(roots).scoped(format!("from import:{name}")),
    ))
}

/// P3 through a library (§3.9): re-export what import `name` brought, from the library it was
/// imported into, with the options and the `group by` field `index` records. The collection
/// that comes back is byte for byte the one `index` describes when the round trip is exact;
/// what is compared is the files and the index's `files` and `rsd`, not its scope's text.
///
/// The snapshot is the library's log **and everything the import retained**: what it staged,
/// what waits and what it quarantined, as well as what it accepted. That is P1's import-side
/// set (§3.9), and it makes P3 the same statement about transport P1 is. Over the log alone,
/// a collection holding one unit a check refuses could never come back whole, though every
/// byte of it arrived and was kept.
pub fn reexport_import(
    lib: &(impl LibraryWrite + Sized),
    name: &str,
    index: &crate::index::ExportIndex,
) -> Result<crate::export::Collection, LibError> {
    let opts =
        crate::export::ExportOptions::from_index(index, lib.snapshot()).ok_or_else(|| {
            LibError::Query(
                "the index names a layout, closure or flag this build does not know".into(),
            )
        })?;
    let base = crate::export::snapshot(lib, &opts.with)?;
    let snap = Store::from_records(
        base.iter()
            .cloned()
            .chain(crate::import::retained(lib, name)?)
            .collect(),
    );
    let mut sel = import_scope(lib, name)?
        .ok_or_else(|| LibError::Query(format!("import:{name} holds no export view to resolve")))?;
    if let Some(field) = index.flags.iter().find_map(|f| f.strip_prefix("group-by=")) {
        sel = group(&snap, sel, field)?;
    }
    crate::export::export_library(lib, &snap, &sel, &opts)
}

/// The import a scope names, when it is exactly `from import:<name>` and nothing else.
pub fn names_an_import(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("from")?.trim_start();
    let name = rest.strip_prefix("import:")?.trim_end();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::names_an_import;

    #[test]
    fn only_a_bare_import_scope_names_an_import() {
        assert_eq!(names_an_import("from import:bible"), Some("bible"));
        assert_eq!(names_an_import("  from   import:a/b \n"), Some("a/b"));
        assert_eq!(
            names_an_import("from import:bible\nwhere status = asserted"),
            None
        );
        assert_eq!(names_an_import("from library"), None);
        assert_eq!(names_an_import("from import:"), None);
    }
}
