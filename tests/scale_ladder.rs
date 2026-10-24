//! The scale ladder (SMYSL-2.8 §5.4): the §2.8 in-process stages on the `graph` corpus.
//!
//! ```text
//! cargo test --release --test scale_ladder -- --ignored --nocapture
//! SMYSL_SCALE=31102,155510 cargo test --release --test scale_ladder -- --ignored --nocapture
//! ```
//!
//! A measurement, not a gate, for the reason every `scaling.rs` in the workspace gives: timing
//! assertions on shared runners fail for reasons that have nothing to do with the code. It is
//! the baseline TX-P13a's per-step "within ±5%" is read against, and it prints a digest of every
//! derived thread so a port can be shown to change no bytes.
//!
//! **The corpus is §5.4's shape, not its bytes.** `gen_graph.py` writes surface text seeded by
//! Python's `random`; this builds the records directly with a fixed LCG. Same proportions: `n`
//! ten-word `prose` units (`cited`, a `doc` source), one `claim` (`derived`, `grounds` on three
//! earlier prose units) per ten, one `backs` from every claim after the first to an earlier one.
//! The surface step also writes a label binding per unit; this does not, so records are units
//! plus relations.

use std::time::Instant;

use smysl_core::{
    canonical_uid, hash_bytes, to_cbor, AgentId, KernelType, Record, RelKind, Relation, SourceKind,
    SourceRef, Status, ThreadId, ThreadSchema, Uid, UnitCoreBuilder,
};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_retrieve::{Query, Retriever};

const VOC: &str = "and the of to that in he shall unto for i his a lord they be is him not them \
                   it with all thou thy was god which my me said but ye their have thee from as \
                   are when this out were by you up made one king people house come day earth \
                   land son father";

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as usize
    }
}

/// `n` prose units, a claim per ten, a `backs` per claim after the first.
pub fn graph(n: usize) -> Vec<Record> {
    let voc: Vec<&str> = VOC.split_whitespace().collect();
    let mut rng = Lcg(7);
    let sentence = |rng: &mut Lcg| {
        let mut w: Vec<String> = (0..10)
            .map(|_| voc[rng.below(voc.len())].to_string())
            .collect();
        w[0] = w[0][..1].to_uppercase() + &w[0][1..];
        format!("{}.", w.join(" "))
    };
    let mut records = Vec::new();
    let mut prose: Vec<Uid> = Vec::new();
    let mut claims: Vec<Uid> = Vec::new();
    for i in 0..n {
        let text = sentence(&mut rng);
        let u = UnitCoreBuilder::new(KernelType::Prose, text, Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, format!("bench:g#{i}")))
            .build()
            .unwrap();
        prose.push(canonical_uid(&u));
        records.push(Record::Unit(u));
        if i % 10 == 9 {
            let mut g: Vec<Uid> = (0..3).map(|_| prose[rng.below(i + 1)]).collect();
            g.sort();
            g.dedup();
            let text = sentence(&mut rng);
            let c = UnitCoreBuilder::new(KernelType::Claim, text, Status::Derived)
                .grounds(g)
                .build()
                .unwrap();
            claims.push(canonical_uid(&c));
            records.push(Record::Unit(c));
        }
    }
    for c in 1..claims.len() {
        let to = claims[rng.below(c)];
        records.push(Record::Relation(Relation::new(
            RelKind::Backs,
            claims[c],
            to,
        )));
    }
    records
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Median of three.
fn median(f: impl Fn() -> f64) -> f64 {
    let mut v = [f(), f(), f()];
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[1]
}

fn short(h: [u8; 32]) -> String {
    h[..6].iter().map(|b| format!("{b:02x}")).collect()
}

const SCHEMAS: [ThreadSchema; 5] = [
    ThreadSchema::Analysis,
    ThreadSchema::Narrative,
    ThreadSchema::Brief,
    ThreadSchema::Qa,
    ThreadSchema::Plan,
];

#[test]
#[ignore = "a measurement, not a gate"]
fn ladder() {
    let sizes: Vec<usize> = std::env::var("SMYSL_SCALE")
        .unwrap_or_else(|_| "31102,155510".into())
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    for n in sizes {
        let records = graph(n);
        let units = records
            .iter()
            .filter(|r| matches!(r, Record::Unit(_)))
            .count();
        let bytes: Vec<u8> = records.iter().flat_map(to_cbor).collect();
        println!(
            "\ngraph-{n}: {units} units, {} relations, {} records, {:.1} MB CBOR",
            records.len() - units,
            records.len(),
            bytes.len() as f64 / 1e6
        );
        let t = Instant::now();
        let (decoded, _) = smysl_core::from_cbor_seq(&bytes).unwrap();
        println!("  {:<28} {:>9.1} ms", "from_cbor_seq", ms(t));
        let load = median(|| {
            let t = Instant::now();
            std::hint::black_box(Store::from_records(decoded.clone()));
            ms(t)
        });
        println!("  {:<28} {:>9.1} ms", "Store::from_records", load);
        let store = Store::from_records(decoded);

        let t = Instant::now();
        let report = smysl_check::check(&store, smysl_check::CheckOptions::default());
        println!(
            "  {:<28} {:>9.1} ms  ({} diagnostics)",
            "check",
            ms(t),
            report.diagnostics.len()
        );
        let t = Instant::now();
        let sal = salience(&store, &SalienceRequest::default());
        println!("  {:<28} {:>9.1} ms", "salience", ms(t));
        let mut req = smysl_pack::PackRequest::default();
        req.budget = 4000;
        let t = Instant::now();
        let packed = smysl_pack::pack(&store, &sal, &req).unwrap();
        println!(
            "  {:<28} {:>9.1} ms  ({} units)",
            "pack (budget 4000)",
            ms(t),
            packed.selection.len()
        );
        for schema in SCHEMAS {
            let opts = smysl_thread::DeriveOptions::new(
                ThreadId::new("t/ladder").unwrap(),
                AgentId::new("tool:ladder").unwrap(),
            );
            let t = Instant::now();
            let (thread, _) = smysl_thread::derive_thread(&store, schema, &opts);
            let took = ms(t);
            println!(
                "  {:<28} {:>9.1} ms  thread {}",
                format!("derive_thread ({schema:?})"),
                took,
                short(hash_bytes(&to_cbor(&Record::Thread(thread))))
            );
        }
        let t = Instant::now();
        let bm = smysl_retrieve::Bm25::index(&store);
        println!("  {:<28} {:>9.1} ms", "Bm25::index", ms(t));
        let t = Instant::now();
        let hits = bm.search(&Query::new("lord king", 10));
        println!(
            "  {:<28} {:>9.1} ms  ({} hits)",
            "Bm25 search",
            ms(t),
            hits.len()
        );
        let one = UnitCoreBuilder::new(KernelType::Prose, "One more.", Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, "bench:g#append"))
            .build()
            .unwrap();
        let mut grown = store.clone();
        let t = Instant::now();
        grown.append(&[Record::Unit(one)]).unwrap();
        println!("  {:<28} {:>9.1} ms", "append 1 record", ms(t));
        // The adjacency is lazy since TX-P13a step 3: the first read after an append pays for
        // it, and this is that bill, so the append line above is not read as the whole cost.
        let t = Instant::now();
        grown.prepare();
        println!("  {:<28} {:>9.1} ms", "first read after it", ms(t));
        // Staging (TX-P13a step 10): fifty new units checked against the store they would join.
        // Until that step this copied every record of the store into a new one first.
        let fifty: Vec<smysl_core::UnitCore> = (0..50)
            .map(|i| {
                UnitCoreBuilder::new(
                    KernelType::Prose,
                    format!("Staged unit {i}."),
                    Status::Cited,
                )
                .source(SourceRef::new(SourceKind::Doc, format!("bench:stage#{i}")))
                .build()
                .unwrap()
            })
            .collect();
        let attest = smysl_ingest::Attest::new(
            AgentId::new("tool:ladder").unwrap(),
            smysl_core::Rung::Document,
            smysl_core::Hlc::new(1, 0, AgentId::new("tool:ladder").unwrap()),
        );
        let t = Instant::now();
        let staged =
            smysl_ingest::stage::prepare(&store, fifty, Vec::new(), Default::default(), &attest);
        println!(
            "  {:<28} {:>9.1} ms  ({} records, {} diagnostics)",
            "stage 50 units",
            ms(t),
            staged.records().len(),
            staged.report.diagnostics.len()
        );
        let t = Instant::now();
        std::hint::black_box(store.units_in_observed_order());
        println!("  {:<28} {:>9.1} ms", "units_in_observed_order", ms(t));
        println!(
            "  state {}  record set {}",
            short(store.state_hash()),
            short(store.record_set_digest())
        );
    }
}
