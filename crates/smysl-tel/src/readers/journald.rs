//! `journald/1` — `journalctl -o json`, one entry per line (RFC SMYSL-3 §6, §19.6, §20.1, TL-P9).
//!
//! A log record is a reading (D-T1): each entry becomes one event of a log stream, never a unit.
//!
//! - **Instant:** `__REALTIME_TIMESTAMP`, microseconds since the epoch, as journald writes it
//!   (a decimal string). An entry without one is refused: the record has no time to be kept at.
//! - **Body:** `[MESSAGE, PRIORITY, fields]`. `MESSAGE` is text, `""` when absent; journald
//!   writes a message that is not UTF-8 as an array of byte values, which is read lossily for
//!   the message and kept exactly in `fields`. `PRIORITY` is its number, or null. `fields` is the
//!   §5.5 JCS text of every field whose name does not start `__`, so the cursor, the monotonic
//!   clock and the sequence numbers, which differ between two reads of one entry, are not part
//!   of it: reading the same journal twice gives the same events.
//! - **Stream** (§6): the JCS text of `{host, unit}`, `_HOSTNAME`, and `_SYSTEMD_UNIT`, else
//!   `SYSLOG_IDENTIFIER`, else `""`.

use crate::jcs;
use crate::json::{self, Json};
use crate::readers::{shape, ReadError};

/// The reader id.
pub const ID: &str = "journald/1";

/// One journal entry, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub realtime_us: u64,
    pub message: String,
    pub priority: Option<u8>,
    pub host: String,
    pub unit: String,
    /// The JCS text of the fields kept.
    pub fields: String,
}

impl Entry {
    /// The key of the log stream it belongs to (§6).
    pub fn stream_key(&self) -> String {
        jcs::canonical(&Json::Object(vec![
            ("host".to_string(), Json::String(self.host.clone())),
            ("unit".to_string(), Json::String(self.unit.clone())),
        ]))
        .0
    }

    /// Its event body (§20.1).
    pub fn body(&self) -> Vec<u8> {
        let mut e = smysl_core::cbor::writer::Enc::new();
        e.array_head(3);
        e.text(&self.message);
        match self.priority {
            Some(p) => e.uint(u64::from(p)),
            None => e.raw(&[smysl_core::cbor::NULL]),
        }
        e.text(&self.fields);
        e.into_bytes()
    }

    /// The instant, in nanoseconds.
    pub fn unix_nano(&self) -> u64 {
        self.realtime_us.saturating_mul(1_000)
    }
}

/// The message of a journald body, for Drain.
pub fn message_of(body: &[u8]) -> Option<String> {
    let mut d = smysl_core::cbor::reader::Dec::new(body);
    if d.array_head().ok()? != 3 {
        return None;
    }
    d.text().ok().map(str::to_string)
}

fn text_of(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        // A field that is not UTF-8: its bytes, as an array of numbers.
        Json::Array(items) => {
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|i| match i {
                    Json::Number(n) => n.parse::<u8>().ok(),
                    _ => None,
                })
                .collect();
            bytes.map(|b| String::from_utf8_lossy(&b).into_owned())
        }
        _ => None,
    }
}

/// Read `journalctl -o json` output.
pub fn read(text: &str) -> Result<Vec<Entry>, ReadError> {
    let mut out = Vec::new();
    for (n, entry) in json::parse_lines(text)?.into_iter().enumerate() {
        let Json::Object(members) = &entry else {
            return Err(shape(format!("entry {}: not an object", n + 1)));
        };
        let get = |k: &str| entry.get(k).and_then(text_of);
        let realtime_us = get("__REALTIME_TIMESTAMP")
            .and_then(|t| t.parse::<u64>().ok())
            .ok_or_else(|| shape(format!("entry {}: no __REALTIME_TIMESTAMP", n + 1)))?;
        let priority = match get("PRIORITY") {
            None => None,
            Some(p) => Some(
                p.parse::<u8>()
                    .ok()
                    .filter(|p| *p <= 7)
                    .ok_or_else(|| shape(format!("entry {}: PRIORITY {p:?}", n + 1)))?,
            ),
        };
        let kept: Vec<(String, Json)> = members
            .iter()
            .filter(|(k, _)| !k.starts_with("__"))
            .cloned()
            .collect();
        out.push(Entry {
            realtime_us,
            message: get("MESSAGE").unwrap_or_default(),
            priority,
            host: get("_HOSTNAME").unwrap_or_default(),
            unit: get("_SYSTEMD_UNIT")
                .or_else(|| get("SYSLOG_IDENTIFIER"))
                .unwrap_or_default(),
            fields: jcs::canonical(&Json::Object(kept)).0,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_reads_of_one_entry_are_one_body_and_the_cursor_is_not_identity() {
        let a = r#"{"__CURSOR":"s=1;i=1","__REALTIME_TIMESTAMP":"1768557600000123","__MONOTONIC_TIMESTAMP":"5","_HOSTNAME":"h1","_SYSTEMD_UNIT":"cart.service","PRIORITY":"6","MESSAGE":"connection from 10.0.0.7 closed after 41 ms"}"#;
        let b = a.replace("s=1;i=1", "s=9;i=9").replace(r#""5""#, r#""77""#);
        let (x, y) = (read(a).unwrap(), read(&b).unwrap());
        assert_eq!(x[0].body(), y[0].body());
        assert_eq!(x[0].stream_key(), r#"{"host":"h1","unit":"cart.service"}"#);
        assert_eq!(x[0].unix_nano() % 1_000_000, 123_000);
        assert_eq!(
            message_of(&x[0].body()).as_deref(),
            Some("connection from 10.0.0.7 closed after 41 ms")
        );
    }

    #[test]
    fn a_message_that_is_not_utf8_is_read_lossily_and_kept_exactly() {
        let t =
            r#"{"__REALTIME_TIMESTAMP":"1","SYSLOG_IDENTIFIER":"kernel","MESSAGE":[104,105,255]}"#;
        let e = &read(t).unwrap()[0];
        assert_eq!(e.message, "hi\u{fffd}");
        assert_eq!(e.unit, "kernel");
        assert!(e.fields.contains("[104,105,255]"), "{}", e.fields);
        assert_eq!(e.priority, None);
        assert!(read(r#"{"MESSAGE":"no time"}"#).is_err());
        assert!(read(r#"{"__REALTIME_TIMESTAMP":"1","PRIORITY":"9"}"#).is_err());
    }
}
