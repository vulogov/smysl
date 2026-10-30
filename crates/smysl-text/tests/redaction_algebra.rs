//! Rule Z under merge: `P-Z1`–`P-Z4` (SMYSL-2.4 §5.3, TX-P2 step 4).
//!
//! # Why this is here and not in `smysl-graph`
//!
//! §5.3 puts this harness in `crates/smysl-graph/tests/redaction_algebra.rs`, and two of its
//! four properties cannot be stated there. `P-Z2` and `P-Z3` are about records 15 and 18 — and
//! a `Store` never holds one: `Store::append` refuses every part text and reading with
//! `SMY-E452`, redacted or not (OQ-39). So in a store the two properties are vacuously true,
//! and the place they have content is the **object store**, which is this crate's.
//!
//! What that moves is the harness, not the claim. A "peer" here is a whole library — a catalog
//! and its objects — and a merge is what one really is for a library: the other peer's records
//! appended, and then the library opened, which is where rule Z is enforced.
//!
//! # The four properties
//!
//! | id | claim |
//! |---|---|
//! | `P-Z1` | the merged **record set** does not depend on the order peers merged in |
//! | `P-Z2` | after any merge sequence, no object exists for a part any participant redacted |
//! | `P-Z3` | re-merging with a stale peer never restores a redacted part |
//! | `P-Z4` | a redaction removes only text: manifests, units and relations are untouched |

#![cfg(feature = "reader-txt")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::ids::{AgentId, Tid};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{Carry, Record};
use smysl_core::{KernelType, LangTag, Status, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;

/// A seeded xorshift, so a failure is reproducible from its seed alone — as
/// `merge_algebra.rs` has it, for the same reason.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-redaction-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("a scratch directory");
        Scratch(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The nth text a peer may hold. Distinct bytes, so distinct tids.
fn text_of(n: usize) -> String {
    format!("part number {n}\nthe second line of part {n}\nand a third, so the reader has rows\n")
}

fn spec(n: usize) -> AddSpec {
    AddSpec::new("txt/1", format!("text-{n}"), "CC0-1.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text)
}

/// Add texts `which` to a fresh library, and return it with the tids in order.
fn peer(tag: &str, which: &[usize]) -> (Scratch, Vec<Tid>) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("created");
    let mut tids = Vec::new();
    for n in which {
        let body = text_of(*n);
        let added = lib
            .add(&Input::new(body.as_bytes()), &spec(*n), &Caps::DEFAULT)
            .expect("added");
        tids.push(added.parts[0]);
    }
    (dir, tids)
}

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

fn redact(lib: &mut Library, tid: &Tid) {
    lib.redact(tid, &agent(), Hlc::new(1_726_500_000_000, 0, agent()), None)
        .expect("redacted");
}

/// Every record in a library's catalog.
fn records_of(root: &Path) -> Vec<Record> {
    let lib = Library::open(root).expect("opened");
    lib.store().iter().cloned().collect()
}

/// Whether a library holds any object at all for this part.
///
/// Both halves: the part and every reading a manifest names of it. Rule Z is about 15 **and**
/// 18, and a reading left behind is a map of offsets and speakers into bytes nobody may hold.
fn holds_anything(lib: &Library, tid: &Tid) -> bool {
    if lib.objects().has_part(tid) {
        return true;
    }
    lib.store().manifests().any(|(_, m)| {
        m.parts
            .iter()
            .any(|e| &e.tid == tid && lib.objects().has_reading(&e.rdid))
    })
}

/// Merge `from`'s catalog into `into`, the way a library merges: append, then open.
///
/// Opening is not an afterthought — it is where rule Z runs, so "merge" for a library *is*
/// these two steps. A caller that appended and did not reopen would hold a catalog that says a
/// part is redacted beside the bytes, which is the state `Library::open` exists to end.
fn merge_into(into: &Path, from: &Path) {
    let records = records_of(from);
    let mut lib = Library::open(into).expect("opened");
    lib.append_records(&records).expect("appended");
    drop(lib);
    Library::open(into).expect("reopened");
}

/// `P-Z1`: the merged record set does not depend on the order.
///
/// Over the records themselves rather than over the libraries, because that is where the claim
/// lives: rule U says merge is a union, and a redaction is an ordinary record in it. The digest
/// is `Store::state_hash`, which `merge_algebra.rs` already establishes ignores log order and
/// notices content.
#[test]
fn pz1_the_merged_record_set_is_order_independent() {
    let mut rng = Rng(0x2026_1009_0001);
    for round in 0..12 {
        let (a, a_tids) = peer("pz1-a", &[1, 2]);
        let (b, b_tids) = peer("pz1-b", &[2, 3]);
        {
            let mut lib = Library::open(a.path()).expect("opened");
            redact(&mut lib, &a_tids[round % a_tids.len()]);
        }
        {
            let mut lib = Library::open(b.path()).expect("opened");
            redact(&mut lib, &b_tids[rng.below(b_tids.len())]);
        }
        let left = records_of(a.path());
        let right = records_of(b.path());

        let mut forward = left.clone();
        forward.extend(right.clone());
        let mut backward = right;
        backward.extend(left);
        let one = Store::from_records(forward.clone());
        let other = Store::from_records(backward);
        assert_eq!(
            one.state_hash(),
            other.state_hash(),
            "round {round}: the union depends on the order"
        );
        // Idempotent: the same records twice are the same store.
        let mut twice = forward.clone();
        twice.extend(forward);
        assert_eq!(Store::from_records(twice).state_hash(), one.state_hash());
        // And the redactions survived into the union, which is what makes the rest testable.
        assert!(one.redacted_count() >= 1, "round {round}");
    }
}

/// `P-Z2`: after any merge sequence, no object exists for a part any participant redacted.
///
/// The property rule Z is for, and the one that needs an object store to mean anything. Peer A
/// redacts a part peer B still holds; B merges A; B must no longer hold it.
#[test]
fn pz2_no_object_survives_a_redaction_from_any_peer() {
    let (a, a_tids) = peer("pz2-a", &[1, 2]);
    let (b, b_tids) = peer("pz2-b", &[1, 2, 3]);
    assert_eq!(a_tids[0], b_tids[0], "the same text is the same part");

    {
        let mut lib = Library::open(a.path()).expect("opened");
        redact(&mut lib, &a_tids[0]);
    }
    // B has not heard. It holds the bytes, and correctly so.
    {
        let lib = Library::open(b.path()).expect("opened");
        assert!(
            holds_anything(&lib, &b_tids[0]),
            "B holds it before merging"
        );
    }

    merge_into(b.path(), a.path());

    let lib = Library::open(b.path()).expect("opened");
    assert!(
        !holds_anything(&lib, &b_tids[0]),
        "a redaction from a peer left the bytes behind"
    );
    assert!(lib.store().is_redacted(&b_tids[0]));
    // And only that part: the other two are untouched.
    for tid in &b_tids[1..] {
        assert!(holds_anything(&lib, tid), "{} should still be held", tid);
    }
}

/// `P-Z3`: re-merging with a stale peer never restores a redacted part.
///
/// Twice over, because there are two ways the bytes could come back. A stale peer's *records*
/// cannot bring them — a log holds no text — so the real risk is the file: somebody re-runs
/// `text add` on the source the part came from. Rule Z withholds the objects and says so, and
/// the manifest is still written, which is the asymmetry the spec's "drop, do not refuse"
/// sentence is about.
#[test]
fn pz3_a_stale_peer_cannot_restore_a_redacted_part() {
    let (a, a_tids) = peer("pz3-a", &[1]);
    let (stale, stale_tids) = peer("pz3-stale", &[1]);
    assert_eq!(a_tids[0], stale_tids[0]);

    {
        let mut lib = Library::open(a.path()).expect("opened");
        redact(&mut lib, &a_tids[0]);
    }
    // The stale peer has the part and no redaction. Merge it in, repeatedly.
    for _ in 0..3 {
        merge_into(a.path(), stale.path());
        let lib = Library::open(a.path()).expect("opened");
        assert!(
            !holds_anything(&lib, &a_tids[0]),
            "merging a stale peer restored the bytes"
        );
    }

    // And the other way the bytes could return: adding the source file again.
    let body = text_of(1);
    let mut lib = Library::open(a.path()).expect("opened");
    let again = lib
        .add(&Input::new(body.as_bytes()), &spec(1), &Caps::DEFAULT)
        .expect("the add is not refused");
    assert_eq!(again.objects_written, 0, "nothing was written");
    assert_eq!(
        again.objects_redacted, 2,
        "the part and its reading were withheld"
    );
    assert!(!holds_anything(&lib, &a_tids[0]));
    // The manifest is there, and asking for the passage says why there is no text.
    assert!(lib.store().manifest(&again.mid).is_some());
    let err = lib.part(&a_tids[0]).expect_err("refused");
    assert!(err.to_string().contains("redacted"), "{err}");
}

/// `P-Z4`: a redaction removes only text.
///
/// Everything else in the catalog is identical with and without it — the manifests that name
/// the part, the units drawn from it, their uids and spans. A redaction is not a retraction,
/// and a corpus that quietly dropped the claims made from a text would be answering a demand by
/// falsifying its own history.
#[test]
fn pz4_a_redaction_removes_only_text() {
    let (dir, tids) = peer("pz4", &[1, 2]);
    // A unit whose source names the part, as ingest would write it.
    // `Speculative`, because rule M requires grounds for `inferred` and the status is not what
    // this property is about.
    let unit = UnitCoreBuilder::new(
        KernelType::Claim,
        "the first part says so",
        Status::Speculative,
    )
    .source(smysl_core::types::SourceRef::new(
        smysl_core::types::SourceKind::Doc,
        format!("{}#L1", tids[0].canonical()),
    ))
    .build()
    .expect("a unit");
    let uid = smysl_core::canonical_uid(&unit);

    let before: BTreeSet<Vec<u8>> = {
        let mut lib = Library::open(dir.path()).expect("opened");
        lib.append_records(&[Record::Unit(unit.clone())])
            .expect("appended");
        lib.store().iter().map(smysl_core::to_cbor).collect()
    };

    {
        let mut lib = Library::open(dir.path()).expect("opened");
        redact(&mut lib, &tids[0]);
    }

    let lib = Library::open(dir.path()).expect("opened");
    let after: BTreeSet<Vec<u8>> = lib.store().iter().map(smysl_core::to_cbor).collect();
    let added: Vec<&Vec<u8>> = after.difference(&before).collect();
    assert_eq!(added.len(), 1, "exactly one record was added");
    assert!(
        before.is_subset(&after),
        "a redaction removed a record from the log"
    );

    // The unit, its source and the manifests are all still there and unchanged.
    assert!(
        lib.store().units().any(|(u, _)| *u == uid),
        "the unit stayed"
    );
    assert_eq!(lib.store().manifest_count(), 2, "both manifests stayed");
    assert!(lib.store().units_with_tid(&tids[0]).contains(&uid));
    // What went is the text, and only of the redacted part.
    assert!(!holds_anything(&lib, &tids[0]));
    assert!(holds_anything(&lib, &tids[1]));
}
