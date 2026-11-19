//! `cloudevents/1` — CloudEvents 1.0 in the JSON structured mode (RFC SMYSL-3 §6, §19.6, §20.1,
//! TL-P9): one event, a batch (a JSON array), or JSON lines.
//!
//! - **Required:** `specversion` `1.0`, `id`, `source` and `type`, as CloudEvents requires, and
//!   `time`, which it does not: an event with no time has no instant to be kept at, so it is
//!   refused rather than given one.
//! - **Instant:** `time`, RFC 3339, to the nanosecond.
//! - **Body:** `[JCS text of the event]`, the whole event. CloudEvents makes (`source`, `id`)
//!   unique, so a producer's retry is the same event, and reading it twice adds nothing.
//! - **Stream** (§6): the JCS text of `{source, type}`.
//! - **Message,** for Drain (§21): `type`, then `subject` when there is one.

use crate::jcs;
use crate::json::{self, Json};
use crate::readers::{shape, ReadError};

/// The reader id.
pub const ID: &str = "cloudevents/1";

/// One event, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudEvent {
    pub source: String,
    pub ty: String,
    pub subject: Option<String>,
    pub unix_nano: u64,
    /// The JCS text of the whole event.
    pub text: String,
}

impl CloudEvent {
    /// The key of the event stream it belongs to (§6).
    pub fn stream_key(&self) -> String {
        jcs::canonical(&Json::Object(vec![
            ("source".to_string(), Json::String(self.source.clone())),
            ("type".to_string(), Json::String(self.ty.clone())),
        ]))
        .0
    }

    /// Its event body (§20.1).
    pub fn body(&self) -> Vec<u8> {
        let mut e = smysl_core::cbor::writer::Enc::new();
        e.array_head(1);
        e.text(&self.text);
        e.into_bytes()
    }
}

/// The message of a CloudEvents body, for Drain: `type`, then `subject`.
pub fn message_of(body: &[u8]) -> Option<String> {
    let mut d = smysl_core::cbor::reader::Dec::new(body);
    if d.array_head().ok()? != 1 {
        return None;
    }
    let ev = json::parse(d.text().ok()?).ok()?;
    let ty = ev.get("type")?.as_str()?;
    Some(match ev.get("subject").and_then(Json::as_str) {
        Some(s) => format!("{ty} {s}"),
        None => ty.to_string(),
    })
}

/// Read one event, a batch, or JSON lines.
pub fn read(text: &str) -> Result<Vec<CloudEvent>, ReadError> {
    let docs = match json::parse(text) {
        Ok(Json::Array(items)) => items,
        Ok(one) => vec![one],
        Err(_) => json::parse_lines(text)?,
    };
    docs.iter()
        .enumerate()
        .map(|(n, d)| event(d).map_err(|e| shape(format!("event {}: {e}", n + 1))))
        .collect()
}

fn event(d: &Json) -> Result<CloudEvent, String> {
    if !matches!(d, Json::Object(_)) {
        return Err("not an object".into());
    }
    let s = |k: &str| d.get(k).and_then(Json::as_str);
    match s("specversion") {
        Some("1.0") => {}
        other => return Err(format!("specversion {other:?}, not \"1.0\"")),
    }
    let req = |k: &str| {
        s(k).filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("no {k}"))
    };
    let (_, source, ty) = (req("id")?, req("source")?, req("type")?);
    let time = req("time")?;
    let unix_nano = rfc3339_nanos(&time).ok_or_else(|| format!("time {time:?} is not RFC 3339"))?;
    Ok(CloudEvent {
        source,
        ty,
        subject: s("subject").map(str::to_string),
        unix_nano,
        text: jcs::canonical(d).0,
    })
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 `date-time` to nanoseconds since the epoch; `None` for anything else, or a time
/// before 1970.
pub fn rfc3339_nanos(t: &str) -> Option<u64> {
    let b = t.as_bytes();
    let num = |r: core::ops::Range<usize>| -> Option<i64> {
        let s = t.get(r)?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let digits = t.get(start..i)?;
        if digits.is_empty() {
            return None;
        }
        let ns: String = digits
            .chars()
            .chain(std::iter::repeat('0'))
            .take(9)
            .collect();
        frac = ns.parse().ok()?;
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        c @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            let o = (oh * 60 + om) * 60;
            if *c == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + s.min(59) - offset;
    u64::try_from(secs)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(frac as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_is_read_to_the_nanosecond_with_its_offset() {
        assert_eq!(rfc3339_nanos("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_nanos("2026-01-16T10:00:00.000123456Z"),
            Some(1_768_557_600_000_123_456)
        );
        assert_eq!(
            rfc3339_nanos("2026-01-16T11:00:00.5+01:00"),
            Some(1_768_557_600_500_000_000)
        );
        for bad in [
            "2026-01-16",
            "2026-13-01T00:00:00Z",
            "2026-01-16T10:00:00",
            "x",
        ] {
            assert_eq!(rfc3339_nanos(bad), None, "{bad}");
        }
    }

    #[test]
    fn one_a_batch_and_lines_read_alike_and_a_retry_is_one_event() {
        let one = r#"{"specversion":"1.0","id":"1","source":"/argocd","type":"dev.cdevents.service.deployed.0.1.1","subject":"cart","time":"2026-01-16T10:00:00Z","data":{"version":"v2"}}"#;
        let batch = format!("[{one}]");
        let lines = format!("{one}\n{one}\n");
        let a = read(one).unwrap();
        assert_eq!(a, read(&batch).unwrap());
        let l = read(&lines).unwrap();
        assert_eq!((l.len(), &l[0]), (2, &a[0]));
        assert_eq!(a[0].body(), l[1].body(), "a retry is the same event");
        assert_eq!(
            message_of(&a[0].body()).as_deref(),
            Some("dev.cdevents.service.deployed.0.1.1 cart")
        );
        assert!(read(&one.replace(r#","time":"2026-01-16T10:00:00Z""#, "")).is_err());
        assert!(read(&one.replace(r#""1.0""#, r#""0.3""#)).is_err());
    }
}
