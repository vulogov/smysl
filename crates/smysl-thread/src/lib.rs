//! `smysl-thread` - the five thread schemas and deterministic derivation (§19).
//!
//! The schema set is closed in 0.1 (D-4): the role-assignment rule language is not stable
//! enough to freeze as a user-facing surface. HJSON-defined schemas are deferred to 0.2.
//!
//! [`ThreadSchema`] and [`Role`] are wire format and therefore live in `smysl-core`; what
//! belongs here is the *rule table* that turns a graph into a thread.
//!
//! Filled by SM-P11.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::{Role, Step, Thread, ThreadSchema};

/// Role weights feed the `w_t` term of salience (§1.5). A schema that weighted every role
/// equally would make the term inert, so the tables are non-uniform by construction.
pub fn role_weight(schema: ThreadSchema, role: Role) -> f32 {
    if !schema.allows(role) {
        return 0.0;
    }
    // Until SM-P11 supplies the per-schema tables, the first role of a schema carries the
    // most weight and the rest decay linearly: a placeholder that is at least ordered.
    let n = schema.roles().len() as f32;
    let i = schema
        .roles()
        .iter()
        .position(|r| *r == role)
        .expect("role is allowed by the schema") as f32;
    smysl_core::quantise(1.0 - (i / n) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_set_is_closed_at_five() {
        assert_eq!(ThreadSchema::ALL.len(), 5);
    }

    #[test]
    fn schema_names_round_trip() {
        for &s in ThreadSchema::ALL {
            assert_eq!(ThreadSchema::parse(s.as_str()), Some(s));
        }
        assert_eq!(ThreadSchema::parse("essay"), None);
    }

    #[test]
    fn a_role_outside_its_schema_carries_no_weight() {
        assert_eq!(role_weight(ThreadSchema::Brief, Role::Coda), 0.0);
        assert!(role_weight(ThreadSchema::Brief, Role::BottomLine) > 0.0);
    }

    #[test]
    fn role_weights_are_quantised_and_ordered_within_a_schema() {
        for &s in ThreadSchema::ALL {
            let mut prev = f32::INFINITY;
            for &r in s.roles() {
                let w = role_weight(s, r);
                assert!(smysl_core::quantise(w) == w, "{s}/{r} is not quantised");
                assert!(w < prev, "{s}/{r} must weigh less than the role before it");
                prev = w;
            }
        }
    }
}
