//! Export round trips (SMYSL-2.7 §5.3, TX-P11 step 11): P1 and P3 over generated stores.
//!
//! A generated store is exported whole under a layout, a closure mode and a format the input
//! chooses, and the collection is verified in memory. For `cbor`, P1 and P3 hold; for
//! `surface`, every file reads back as exactly what it can spell (`SMY-E424` never raised).
//! Exporting twice gives the same bytes (rule D). The same properties are asserted over the
//! corpus by `crates/smysl-lib/tests/`; this target changes the search, not the properties.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_fuzz::{generate, Choices};
use smysl_lib::{
    export_store, verify_collection, Closure, ExportOptions, Format, Layout, Selection,
};

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let store = generate(&mut c, 40);
    let layout = match c.below(5) {
        0 => Layout::Single,
        1 => Layout::ByView,
        2 => Layout::ByExpression,
        3 => Layout::ByCollection,
        _ => Layout::ByTime,
    };
    let closure = if c.below(2) == 0 {
        Closure::Full
    } else {
        Closure::Ref
    };
    let surface = c.below(4) == 0;
    let mut opts = ExportOptions::new().layout(layout).closure(closure);
    if surface {
        opts = opts.format(Format::Surface);
    }
    let sel = Selection::all(&store);
    let col = export_store(&store, &sel, &opts).expect("an export of held units does not fail");
    let again = export_store(&store, &sel, &opts).expect("and does not fail twice");
    assert_eq!(
        col, again,
        "the same store and options gave two collections"
    );
    let v = verify_collection(&col);
    assert!(v.holds(), "verify fails on a fresh export: {v:?}");
    if surface {
        assert!(
            col.index.complete,
            "a surface file does not read back: {:?}",
            col.report
        );
    } else {
        assert!(v.p1.holds() && v.p3.holds(), "P1 or P3: {v:?}");
    }
});
