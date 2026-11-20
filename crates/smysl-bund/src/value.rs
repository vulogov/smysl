//! What a script answers.

use std::collections::BTreeMap;

use smysl_core::ids::Uid;

/// A Bund2 value, as the bridge returns it.
///
/// Map keys are in `BTreeMap` order, whatever order Bund2 held them in, so two equal answers
/// compare equal. A STRING that parses as a smysl uid is returned as [`QueryValue::Uid`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum QueryValue {
    /// Bund2's NODATA.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Uid(Uid),
    List(Vec<QueryValue>),
    Map(BTreeMap<String, QueryValue>),
}

impl QueryValue {
    /// Every uid in this value, depth first, in the order they appear.
    pub fn uids(&self) -> Vec<Uid> {
        let mut out = Vec::new();
        let mut work = vec![self];
        while let Some(v) = work.pop() {
            match v {
                QueryValue::Uid(u) => out.push(*u),
                QueryValue::List(items) => work.extend(items.iter().rev()),
                QueryValue::Map(m) => work.extend(m.values().rev()),
                _ => {}
            }
        }
        out
    }
}
