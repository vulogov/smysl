//! The import manifest (SMYSL-2.7 §3.1): record 14, one per file.
//!
//! Its body is a function of the file's bytes and the options only. The alias is
//! `import:<name>`, or `import:<name>/<stem>` when an import holds several files. The licence
//! is `--licence`, `unknown` by default. The origin is `--origin`'s URL, or kind `file` with
//! the **basename**, never a path, so one file imported on two machines has one mid. The
//! identifiers are `original`, the BLAKE3 of the bytes, and `records`, the record-set digest of
//! what they decode to, both in lowercase hex. It carries no parts: the text is not carried
//! until step 6.

use super::ImportOptions;
use crate::export::LibError;
use smysl_core::{LangTag, Manifest, SourceKind, SourceRef};

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The reader id an import manifest records: what read the file was the importer.
pub const READER: &str = "smysl-lib/import/1";

pub(crate) fn build(
    alias: &str,
    file: &str,
    original: &[u8; 32],
    rsd: &[u8; 32],
    opts: &ImportOptions,
) -> Result<Manifest, LibError> {
    if !smysl_core::is_alias(alias) {
        return Err(LibError::BadName(alias.to_string()));
    }
    let mut m = Manifest::new(
        alias,
        LangTag::new("und").expect("a BCP-47 tag"),
        READER,
        opts.licence.clone(),
        smysl_text::part::Policy::default().id(),
    );
    m.identifiers.insert("original".to_string(), hex(original));
    m.identifiers.insert("records".to_string(), hex(rsd));
    m.origin = Some(match &opts.origin {
        Some(url) => SourceRef::new(SourceKind::Url, url.clone()),
        None => SourceRef::new(SourceKind::File, file.to_string()),
    });
    Ok(m)
}
