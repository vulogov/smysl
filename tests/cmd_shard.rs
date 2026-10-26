//! `--shard` (SMYSL-2.8 §4.4; TX-P13c step 1): some of a library's shards read as one store,
//! with a reference into a shard not named waiting on it (`SMY-W500`) rather than dangling.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::text::library::{AddSpec, Library};
use smysl::text::limits::Caps;
use smysl::text::readers::Input;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-shard-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A library of two chats, a reply in one quoting a message in the other.
fn a_library(root: &Path) {
    let mut lib = Library::create(root).unwrap();
    let spec = |alias: &str| {
        AddSpec::new("txt/1", alias, "CC0-1.0")
            .with_lang(smysl::LangTag::new("en").unwrap())
            .with_carry(smysl::Carry::Text)
    };
    let a = lib
        .add(
            &Input::new(b"the reply\n"),
            &spec("chat/replies"),
            &Caps::DEFAULT,
        )
        .unwrap();
    let b = lib
        .add(
            &Input::new(b"the message\n"),
            &spec("chat/messages"),
            &Caps::DEFAULT,
        )
        .unwrap();
    let unit = |tid: &smysl::Tid, gist: &str| {
        let source =
            smysl::SourceRef::new(smysl::SourceKind::Doc, format!("{}#L1", tid.canonical()));
        smysl::UnitCoreBuilder::new(smysl::KernelType::Claim, gist, smysl::Status::Speculative)
            .source(source)
            .build()
            .unwrap()
    };
    let (reply, message) = (
        unit(&a.parts[0], "the reply"),
        unit(&b.parts[0], "the message"),
    );
    let quotes = smysl::Relation::new(
        smysl::RelKind::Extension("x.text/quotes".into()),
        smysl::canonical_uid(&reply),
        smysl::canonical_uid(&message),
    );
    lib.append_records(&[
        smysl::Record::Unit(reply),
        smysl::Record::Unit(message),
        smysl::Record::Relation(quotes),
    ])
    .unwrap();
}

#[test]
fn a_shard_alone_waits_on_the_one_its_reference_names() {
    let dir = scratch("waits");
    a_library(&dir.join("lib"));

    // The whole library: nothing dangles.
    let o = run(&dir, &["check", "-s", "lib"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(!text(&o).contains("SMY-W500"), "{}", text(&o));

    // One shard: the quoted message is in another, and the report says which.
    let o = run(&dir, &["--shard", "chat/replies", "check", "-s", "lib"]);
    let out = text(&o);
    assert!(out.contains("SMY-W500"), "{out}");
    assert!(out.contains("chat/messages"), "{out}");
    assert!(
        !out.contains("SMY-E060"),
        "a waiting reference is not a dangling one: {out}"
    );
    assert!(o.status.success(), "a warning does not fail check: {out}");

    // Both: resolved, and nothing was written to get there.
    let before = std::fs::read(dir.join("lib/log/shards/chat~replies.cbor")).unwrap();
    let o = run(
        &dir,
        &[
            "--shard",
            "chat/replies",
            "--shard",
            "chat/messages",
            "check",
            "-s",
            "lib",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(!text(&o).contains("SMY-W500"), "{}", text(&o));
    assert_eq!(
        std::fs::read(dir.join("lib/log/shards/chat~replies.cbor")).unwrap(),
        before
    );

    // `all` is every shard.
    let o = run(&dir, &["--shard", "all", "check", "-s", "lib"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(!text(&o).contains("SMY-W500"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shard_that_is_not_there_and_a_store_that_is_not_a_library_are_refused() {
    let dir = scratch("refused");
    a_library(&dir.join("lib"));
    let o = run(&dir, &["--shard", "chat/nobody", "check", "-s", "lib"]);
    assert!(!o.status.success());
    let out = text(&o);
    assert!(out.contains("no shard `chat/nobody`"), "{out}");
    assert!(
        out.contains("`chat/replies`"),
        "it lists what there is: {out}"
    );

    std::fs::write(
        dir.join("doc.smy"),
        "@claim c/one { status: speculative }\n~ One.\n",
    )
    .unwrap();
    let o = run(&dir, &["--shard", "all", "check", "-s", "doc.smy"]);
    assert!(!o.status.success());
    assert!(text(&o).contains("not one"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "store-redb")]
#[test]
fn under_the_unit_store_each_shard_is_read_through_its_own() {
    let dir = scratch("redb");
    a_library(&dir.join("lib"));
    let o = run(
        &dir,
        &[
            "--backend",
            "redb",
            "--shard",
            "chat/replies",
            "check",
            "-s",
            "lib",
        ],
    );
    let out = text(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("SMY-W500"), "{out}");
    let o = run(&dir, &["--backend", "redb", "check", "-s", "lib"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(!text(&o).contains("SMY-W500"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&dir);
}
