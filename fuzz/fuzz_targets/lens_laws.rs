//! Lens laws over arbitrary bytes (SMYSL-2.5 §5.3, TX-P12 step 2).
//!
//! An input that decodes as a record sequence is read as one, and each schema declaration in
//! it is checked: so `fixtures/schema/fc9/*.cbor` are seeds as they stand (`make seed-fuzz`).
//! Any other input is split into a key 5 and a payload, and key 5 is offered to
//! `schema::decode` as it arrived. Either way key 5 decodes or is reported (`SMY-W430`), and
//! nothing panics. What decodes is held to:
//!
//! - **L9**: `encode_lenses(decode(bytes)) == bytes`.
//! - **L3**: every contiguous run of decoded steps, cut at every point and composed back, agrees
//!   with the whole run on its hash and on forward application to the payload's view, and on
//!   backward where backward is defined.
//!
//! L1 is not asserted here, on purpose: it is a law about histories rule N admits, and arbitrary
//! steps are not one (a rename onto a key the unit already holds has no inverse on that unit).
//! `crates/smysl-core/tests/lens_laws.rs` generates rule-N histories and holds L1 over them; this
//! target changes the search for panics and for L3 and L9, not the statement of L1.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::schema::{decode, encode_lenses, LensChain, SchemaSet, UnitView};
use smysl_core::surface::payload_to_object;
use smysl_core::types::{SchemaDecl, Status, UnitCoreBuilder};
use smysl_core::{from_cbor_seq, Record, Report, SchemaId};

fuzz_target!(|data: &[u8]| {
    if let Ok((records, _)) = from_cbor_seq(data) {
        for r in &records {
            if let Record::SchemaDecl(d) = r {
                check(d.clone(), &[]);
            }
        }
        return;
    }
    if data.is_empty() {
        return;
    }
    let cut = (data[0] as usize).min(data.len() - 1);
    let (lenses, payload) = data[1..].split_at(cut);
    let mut decl = SchemaDecl::new(SchemaId::parse("x.f/v1").unwrap(), 1);
    decl.types = vec![SchemaId::parse("x.f/t").unwrap()];
    decl.lenses = Some(lenses.to_vec());
    check(decl, payload);
});

fn check(decl: SchemaDecl, payload: &[u8]) {
    let id = decl.id.clone();
    let Some(ty) = decl.types.first().cloned() else {
        return;
    };
    let mut report = Report::new();
    let rev = decode(&decl, &mut report);
    if rev.lenses_opaque {
        return;
    }
    // L9.
    if let Some(raw) = &decl.lenses {
        assert_eq!(&encode_lenses(&rev.lenses), raw, "L9");
    }

    let mut view = UnitView {
        ty: ty.clone(),
        payload: Default::default(),
    };
    if let Ok(o) = payload_to_object(payload) {
        for (k, v) in o.iter() {
            view.payload.insert(k.value.clone(), v.value.clone());
        }
    }

    // L3 over each contiguous run.
    let steps = &rev.lenses;
    let mut start = 0;
    while start < steps.len() {
        let mut end = start + 1;
        while end < steps.len() && steps[end].from == steps[end - 1].to {
            end += 1;
        }
        let run = &steps[start..end];
        let (from, to) = (run[0].from, run[run.len() - 1].to);
        let whole = LensChain::from_steps(id.clone(), from, to, run.to_vec()).unwrap();
        for cut in 0..=run.len() {
            let mid = if cut == run.len() { to } else { run[cut].from };
            let a = LensChain::from_steps(id.clone(), from, mid, run[..cut].to_vec()).unwrap();
            let b = LensChain::from_steps(id.clone(), mid, to, run[cut..].to_vec()).unwrap();
            let ab = a.compose(b).unwrap();
            assert_eq!(ab.hash, whole.hash, "L3 hash");
            let (mut p, mut q) = (view.clone(), view.clone());
            assert_eq!(ab.forward(&mut p), whole.forward(&mut q), "L3 forward");
            assert_eq!(p, q, "L3 forward view");
            let (mut p, mut q) = (view.clone(), view.clone());
            assert_eq!(ab.backward(&mut p), whole.backward(&mut q), "L3 backward");
            assert_eq!(p, q, "L3 backward view");
        }
        start = end;
    }

    // A set of one revision per step's `to`, and inference over the payload's unit: no panics.
    let mut decls = vec![decl.clone()];
    for s in steps {
        let mut d = SchemaDecl::new(id.clone(), s.to);
        d.types = vec![ty.clone()];
        d.lenses = Some(encode_lenses(std::slice::from_ref(s)));
        decls.push(d);
    }
    let set = SchemaSet::from_decls(&decls, &mut report);
    let mut b = UnitCoreBuilder::new(ty, "f", Status::Speculative);
    if let Some(p) = view.payload_bytes() {
        b = b.payload(p);
    }
    if let Ok(core) = b.build() {
        if let Some(g) = set.infer_revision(&core) {
            if let Some(cur) = set.current(&id) {
                let _ = set.chain(&id, g.hi, cur.version);
            }
        }
    }
}
