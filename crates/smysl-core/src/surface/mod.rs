//! Surface syntax (§6, §15.2, §15.3, Appendix A).

pub mod hjson;
pub mod lex;
pub mod parse;
pub mod payload;
pub mod write;

pub use hjson::{parse_object, parse_object_prefix, parse_value, HError, HObject, HValue, Spanned};
pub use lex::{lex, Line, LineClass};
pub use parse::{parse_surface, ParseOutcome};
pub use payload::{object_to_payload, payload_to_object};
pub use write::{write_surface, WriteContext};
