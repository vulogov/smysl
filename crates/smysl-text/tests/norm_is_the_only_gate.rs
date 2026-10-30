//! A tid is over *normalised* bytes, and this is what keeps that true.
//!
//! SMYSL-2.4 §5.1 asks for it as a compile-time property: "`Tid::of` refuses a
//! non-`Normalised` value at compile time". It cannot be one. `smysl-core`'s
//! `Tid::from_normalised_bytes` takes bytes because the decoder has to build a part text out
//! of whatever arrived — including bytes that do *not* hash to the tid beside them, which is
//! the case `SMY-E446` exists to report. A signature that refused those would move one bad
//! record from "reported" to "the store will not open", which is F-12's lesson and was paid
//! for once already.
//!
//! So the guarantee is structural instead, and this is the structure: inside this crate, the
//! bytes-taking constructor is called in exactly two places, both of which have a
//! [`smysl_text::Normalised`] in hand. A source grep is a blunt instrument and that is the
//! point — it fails on the *next* call site, wherever somebody adds one, which is when the
//! question "are these bytes normalised?" is cheap to answer.
//!
//! This is the same technique the purity gate uses for `tokio` and `std::net`, for the same
//! reason: the property is about what the source says, so the check reads the source.

use std::path::Path;

/// The files allowed to name the bytes-taking constructor, and why.
///
/// One file. `ids::tid` is the *public* way to compute a tid and takes a `&Normalised`, but
/// it delegates to `Normalised::tid` rather than calling the constructor itself — so there is
/// exactly one place in this crate where bytes become an identity.
const ALLOWED: &[(&str, &str)] = &[(
    "norm.rs",
    "`Normalised::tid`, which has normalised text by construction",
)];

/// Every `.rs` file under a directory.
fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn the_only_caller_of_from_normalised_bytes_is_the_one_that_holds_normalised_text() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() >= 10, "found only {} source files", files.len());

    let mut offenders = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("readable");
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            // Comments and doc comments may name it; code may not. The same rule the purity
            // gate applies, so the two checks cannot disagree about what counts as a mention.
            let code = line.split("//").next().unwrap_or("");
            if code.contains("from_normalised_bytes")
                && !ALLOWED.iter().any(|(allowed, _)| *allowed == name)
            {
                offenders.push(format!("{name}:{}", n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a tid was computed outside `norm` and `ids`, at {}. If the bytes are normalised, \
         say so with a `Normalised` and call `ids::tid`; if they are not, the tid would name \
         something no other library can reproduce.",
        offenders.join(", ")
    );
}

/// And the allowed file really does call it, so the test is testing something.
///
/// Without this, deleting the call site — or renaming the function upstream — would leave a
/// green test asserting a property nothing holds.
#[test]
fn the_allowed_file_is_the_one_that_calls_it() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (name, why) in ALLOWED {
        let text = std::fs::read_to_string(src.join(name)).expect("readable");
        let calls = text
            .lines()
            .filter(|l| {
                l.split("//")
                    .next()
                    .unwrap_or("")
                    .contains("from_normalised_bytes")
            })
            .count();
        assert!(calls >= 1, "{name} should call it: {why}");
    }
}

/// The property itself, as behaviour: unnormalised bytes and their normalised form do not
/// have the same tid, which is why the gate matters at all.
#[test]
fn unnormalised_bytes_would_name_a_different_part() {
    let normalised = smysl_text::Normalised::of("\u{FEFF}Mark 1:1\r\n");
    assert_eq!(normalised.as_str(), "Mark 1:1\n");
    assert_ne!(
        normalised.tid(),
        smysl_core::Tid::from_normalised_bytes("\u{FEFF}Mark 1:1\r\n".as_bytes()),
        "the same document, two identities — the thing `Normalised` prevents"
    );
}
