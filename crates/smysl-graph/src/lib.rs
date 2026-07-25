//! `smysl-graph` - the append-only store, its derived index, adjacency, merge, lineage,
//! and salience (§16).
//!
//! Pure and synchronous. Every operation here is a bit-reproducible function of its
//! inputs (rule D); the purpose-built adjacency store (D-1) exists precisely so that
//! traversal order is structural rather than defensive.
//!
//! Filled by SM-P3 (store, index, adjacency), SM-P6 (merge), SM-P7 (lineage),
//! SM-P8 (salience).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::error::MergeError;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_pure_and_synchronous() {
        // Placeholder until SM-P3. The real guarantee is enforced by `xtask
        // check-purity`, which greps this crate for network and runtime symbols.
        assert_eq!(
            super::MergeError::ContentionsPresent { count: 0 }.code(),
            None
        );
    }
}
