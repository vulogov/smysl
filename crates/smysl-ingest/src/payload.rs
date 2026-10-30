//! Payload surgery: removing and writing string entries without re-encoding the rest.
//!
//! A unit's payload is deterministic CBOR belonging to an extension schema (rule X), and it is
//! **inside the uid**. Two things follow. Every entry this crate does not understand has to come
//! back out byte for byte — a round trip through the HJSON value model quantises floats, so a
//! consumer that merely *read* a payload to drop one key could move a uid for a key nobody asked
//! about. And the map has to stay sorted by encoded key bytes, as `payload::object_to_payload`
//! writes it, or two encodings of one logical payload would exist.
//!
//! So the entries that stay are copied as the bytes they arrived as, and only the ones written
//! here are encoded. Ungated and `pub(crate)`, because both callers need it and they are on
//! opposite sides of the `text` feature: `text::attach` writes the attribution, and `repair`
//! drops a tool-owned key from a surface answer.

use std::collections::BTreeSet;

/// The payload with the named keys removed and the given string entries written.
///
/// Byte surgery rather than a round trip through the HJSON value model: every other key in the
/// payload belongs to a schema this crate knows nothing about, and re-encoding a value through a
/// model that quantises floats would change a uid for a key nobody asked about. The entries that
/// stay are copied verbatim; the ones written are text, which is what all three of this step's
/// keys are. Sorting is by encoded key bytes, as `payload::object_to_payload` does, so the map is
/// deterministic whichever way it was assembled.
///
/// `None` when nothing is left, because a unit with no unknown keys carries no payload at all
/// rather than an empty map — two encodings of one core would otherwise exist.
pub(crate) fn rewrite(
    payload: Option<&[u8]>,
    drop: &[&str],
    add: &[(&str, String)],
) -> Option<Vec<u8>> {
    use smysl_core::cbor::major;
    use smysl_core::cbor::reader::Dec;
    use smysl_core::cbor::writer::{enc, Enc};

    // Written keys replace rather than duplicate: a map with one key twice is not deterministic
    // CBOR, and the answer's own value for a tool-owned key is exactly what is being overruled.
    let drop: BTreeSet<&str> = drop
        .iter()
        .chain(add.iter().map(|(k, _)| k))
        .copied()
        .collect();
    let mut kept: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    if let Some(bytes) = payload {
        let mut d = Dec::new(bytes);
        let Ok(n) = d.map_head() else {
            // Not a map: the payload is something this code did not write, and a unit's payload
            // is inside its uid. Left exactly as it is, which loses the removal rather than the
            // unit.
            return payload.map(<[u8]>::to_vec);
        };
        for _ in 0..n {
            let Ok(key) = d.text() else {
                return payload.map(<[u8]>::to_vec);
            };
            let keep = !drop.contains(key);
            let encoded = enc(|e| e.text(key));
            let Ok(value) = d.skip_item() else {
                return payload.map(<[u8]>::to_vec);
            };
            if keep {
                kept.push((encoded, value.to_vec()));
            }
        }
    }
    for (key, value) in add {
        kept.push((enc(|e| e.text(key)), enc(|e| e.text(value))));
    }
    if kept.is_empty() {
        return None;
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    let mut e = Enc::new();
    e.head(major::MAP, kept.len() as u64);
    for (k, v) in kept {
        e.raw(&k);
        e.raw(&v);
    }
    Some(e.into_bytes())
}
