//! Information measures over telemetry (RFC SMYSL-5): entropies, mutual information and
//! divergences of a symbolised sample set, each stated as an `x.tel/info` unit.
//!
//! A series becomes symbols in exact decimal arithmetic (§5, the crate's `exact` module); floating point
//! enters once, in the sum over integer counts, through [`num`]; and the result is stored
//! rounded to six places (§6.0), so that a quote of it can be verified by rule TQ.
//!
//! TI-P0 lays down what every statistic needs: the arithmetic in [`num`], the exact decimal
//! operations, and `x.tel/v1` revision 2 with `x.tel/info` (`smysl_core::schemas::tel::INFO`).
//! The grid, the binnings and the statistics arrive with TI-P1 and TI-P2.

pub mod num;
