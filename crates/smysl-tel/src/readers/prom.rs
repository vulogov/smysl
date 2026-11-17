//! `prom/1` — one Prometheus scrape: text format 0.0.4 or OpenMetrics 1.0 (RFC SMYSL-3 §20.1).
//!
//! **One scrape per file, its instant given.** The reader is handed the bytes a target served
//! and the instant they were scraped. A sample with its own timestamp keeps it; any other takes
//! the scrape instant, as Prometheus does.
//!
//! **Which format.** OpenMetrics ends with `# EOF`, and text 0.0.4 has no such line, so the last
//! line decides. The two differ where it matters here: an OpenMetrics timestamp is in seconds and
//! may be fractional, so it is read exactly to the nanosecond; a 0.0.4 timestamp is integer
//! milliseconds.
//!
//! **Values are read exactly** (§5.3): a sample value is a decimal token, read as written. `NaN`
//! and `±Inf` are not values (§5.2 rule 5). They are kept as samples with flag 4, no recorded
//! value, and counted, so the read does not stop and nothing is invented.
//!
//! **Series identity** (§6) is the label set including the metric name, as
//! `name{l1="v1",…}` with labels sorted by name and Prometheus' escaping. The target's own labels
//! (`job`, `instance`) are part of it, and are the caller's to give, since the exposition does not
//! carry them. Where a scraped label has a target label's name, the scraped one is renamed
//! `exported_<name>`, Prometheus' default. An empty label value is the label's absence, as it is
//! to Prometheus.

use std::collections::BTreeMap;

use smysl_core::Decimal;

use crate::readers::{shape, ReadError};

/// The reader id a manifest or recipe records.
pub const ID: &str = "prom/1";

/// Which exposition format a scrape was in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Prometheus text format 0.0.4.
    Text,
    /// OpenMetrics 1.0 text.
    OpenMetrics,
}

/// A metric family's type, from its `# TYPE` line; `Unknown` without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FamilyType {
    Counter,
    Gauge,
    Histogram,
    GaugeHistogram,
    Summary,
    Info,
    StateSet,
    Unknown,
}

impl FamilyType {
    fn parse(s: &str) -> Option<FamilyType> {
        Some(match s {
            "counter" => FamilyType::Counter,
            "gauge" => FamilyType::Gauge,
            "histogram" => FamilyType::Histogram,
            "gaugehistogram" => FamilyType::GaugeHistogram,
            "summary" => FamilyType::Summary,
            "info" => FamilyType::Info,
            "stateset" => FamilyType::StateSet,
            "untyped" | "unknown" => FamilyType::Unknown,
            _ => return None,
        })
    }

    /// The `x.tel/series` kind (§4.3) a series of this family has.
    pub fn kind(self) -> &'static str {
        match self {
            FamilyType::Counter => "counter",
            FamilyType::Histogram | FamilyType::GaugeHistogram => "histogram",
            FamilyType::Summary => "summary",
            FamilyType::Info | FamilyType::StateSet => "state",
            FamilyType::Gauge | FamilyType::Unknown => "gauge",
        }
    }

    /// The leaf class (§4.3) a series of this family has.
    pub fn class(self) -> &'static str {
        match self {
            FamilyType::Counter => "counter",
            FamilyType::Histogram | FamilyType::GaugeHistogram | FamilyType::Summary => "histogram",
            FamilyType::Gauge | FamilyType::Info | FamilyType::StateSet | FamilyType::Unknown => {
                "gauge"
            }
        }
    }
}

/// One sample line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromSample {
    /// The sample's metric name: `http_requests_total`, `latency_bucket`.
    pub name: String,
    /// The family it belongs to: `http_requests`, `latency`.
    pub family: String,
    pub ty: FamilyType,
    /// The sample name's suffix within its family: `_total`, `_bucket`, `_created`, or `""`.
    pub suffix: &'static str,
    /// Labels, the target's included, sorted by name, none empty.
    pub labels: Vec<(String, String)>,
    /// `None` for `NaN` and `±Inf`: not a value (§5.2).
    pub value: Option<Decimal>,
    /// Its own timestamp, if the line had one, as (ms, sub-ms ns).
    pub ts: Option<(u64, Option<u32>)>,
}

impl PromSample {
    /// The series key (§6): `name{l1="v1",…}`, or the bare name with no labels.
    pub fn key(&self) -> String {
        key(&self.name, &self.labels)
    }

    /// Whether a decrease in this series means a counter reset: a counter's samples, and the
    /// `_count` and `_bucket` of a histogram or summary. Never `_sum`, which a negative
    /// observation may lower, and never `_created`.
    pub fn is_monotonic(&self) -> bool {
        match self.ty {
            FamilyType::Counter => self.suffix != "_created",
            FamilyType::Histogram | FamilyType::Summary => {
                matches!(self.suffix, "_count" | "_bucket")
            }
            _ => false,
        }
    }

    /// The labels that make this sample's counter one counter: all of them but `le` and
    /// `quantile`, which split one histogram or summary into its series.
    pub fn group_labels(&self) -> Vec<(String, String)> {
        self.labels
            .iter()
            .filter(|(k, _)| k != "le" && k != "quantile")
            .cloned()
            .collect()
    }
}

/// The series key of `name` with `labels` (sorted): Prometheus' own spelling.
pub fn key(name: &str, labels: &[(String, String)]) -> String {
    if labels.is_empty() {
        return name.to_string();
    }
    let mut out = format!("{name}{{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(k);
        out.push_str("=\"");
        for c in v.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push('}');
    out
}

/// One scrape, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrape {
    pub format: Format,
    pub samples: Vec<PromSample>,
    /// `# HELP` text per family: `x.tel/series-meta`'s, not identity.
    pub help: BTreeMap<String, String>,
    /// `# UNIT` per family (OpenMetrics).
    pub units: BTreeMap<String, String>,
    /// Samples whose value was `NaN` or `±Inf`.
    pub not_values: usize,
}

const SUFFIXES: &[&str] = &[
    "_total", "_created", "_bucket", "_count", "_sum", "_gcount", "_gsum", "_info",
];

/// Read one scrape. `target` is the target's labels (`job`, `instance`, …), which the
/// exposition does not carry and identity needs.
pub fn read(input: &str, target: &[(&str, &str)]) -> Result<Scrape, ReadError> {
    let lines: Vec<&str> = input.lines().collect();
    let last = lines.iter().rev().find(|l| !l.trim().is_empty());
    let format = if last.is_some_and(|l| l.trim_end() == "# EOF") {
        Format::OpenMetrics
    } else {
        Format::Text
    };
    let mut types: BTreeMap<String, FamilyType> = BTreeMap::new();
    let mut out = Scrape {
        format,
        samples: Vec::new(),
        help: BTreeMap::new(),
        units: BTreeMap::new(),
        not_values: 0,
    };
    let at = |n: usize, why: String| shape(format!("line {}: {why}", n + 1));
    for (n, raw) in lines.iter().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            if format == Format::OpenMetrics {
                return Err(at(n, "an empty line in OpenMetrics".into()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let rest = rest.trim_start();
            if rest == "EOF" {
                if lines[n + 1..].iter().any(|l| !l.trim().is_empty()) {
                    return Err(at(n, "text after `# EOF`".into()));
                }
                break;
            }
            let mut parts = rest.splitn(3, ' ');
            let (kw, fam, text) = (parts.next(), parts.next(), parts.next().unwrap_or(""));
            match (kw, fam) {
                (Some("TYPE"), Some(fam)) => {
                    let ty = FamilyType::parse(text.trim())
                        .ok_or_else(|| at(n, format!("`{}` is not a metric type", text.trim())))?;
                    if types.insert(fam.to_string(), ty).is_some() {
                        return Err(at(n, format!("a second `# TYPE` for `{fam}`")));
                    }
                }
                (Some("HELP"), Some(fam)) => {
                    out.help.insert(fam.to_string(), unescape_help(text));
                }
                (Some("UNIT"), Some(fam)) => {
                    out.units.insert(fam.to_string(), text.to_string());
                }
                _ => {}
            }
            continue;
        }
        let s = sample(line, format, &types, target).map_err(|why| at(n, why))?;
        if s.value.is_none() {
            out.not_values += 1;
        }
        out.samples.push(s);
    }
    Ok(out)
}

fn unescape_help(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(c) => out.push(c),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':'
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b':'
}

fn sample(
    line: &str,
    format: Format,
    types: &BTreeMap<String, FamilyType>,
    target: &[(&str, &str)],
) -> Result<PromSample, String> {
    let b = line.as_bytes();
    if b.is_empty() || !is_name_start(b[0]) {
        return Err("not a metric name".into());
    }
    let mut i = 1;
    while i < b.len() && is_name_char(b[i]) {
        i += 1;
    }
    let name = &line[..i];
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    if b.get(i) == Some(&b'{') {
        i += 1;
        loop {
            while b.get(i) == Some(&b' ') {
                i += 1;
            }
            if b.get(i) == Some(&b'}') {
                i += 1;
                break;
            }
            let start = i;
            if !b.get(i).copied().is_some_and(is_name_start) || b[i] == b':' {
                return Err("not a label name".into());
            }
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let lname = &line[start..i];
            while b.get(i) == Some(&b' ') {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                return Err(format!("no `=` after label `{lname}`"));
            }
            i += 1;
            while b.get(i) == Some(&b' ') {
                i += 1;
            }
            if b.get(i) != Some(&b'"') {
                return Err(format!("label `{lname}`'s value is not quoted"));
            }
            i += 1;
            let mut value = String::new();
            loop {
                let rest = &line[i..];
                let mut chars = rest.chars();
                match chars.next() {
                    None => return Err("an unterminated label value".into()),
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('\\') => value.push('\\'),
                            Some('"') => value.push('"'),
                            Some(c) => {
                                return Err(format!("`\\{c}` is not an escape in a label value"))
                            }
                            None => return Err("an unterminated label value".into()),
                        }
                        i += 2;
                    }
                    Some(c) => {
                        value.push(c);
                        i += c.len_utf8();
                    }
                }
            }
            if labels.insert(lname.to_string(), value).is_some() {
                return Err(format!("label `{lname}` twice"));
            }
            while b.get(i) == Some(&b' ') {
                i += 1;
            }
            match b.get(i) {
                Some(b',') => i += 1,
                Some(b'}') => {}
                _ => return Err("labels not separated by `,`".into()),
            }
        }
    }
    let rest = &line[i..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return Err("no value".into());
    }
    // An exemplar (OpenMetrics) follows ` # `; it is not the sample's.
    let rest = match rest.find(" # ") {
        Some(at) if format == Format::OpenMetrics => &rest[..at],
        _ => rest,
    };
    let mut fields = rest.split_whitespace();
    let token = fields.next().ok_or("no value")?;
    let value = match Decimal::parse_token(token) {
        Ok(d) => Some(d),
        Err(smysl_core::DecError::NotAValue) => None,
        Err(e) => return Err(format!("`{token}` is not a sample value: {e}")),
    };
    let ts = match fields.next() {
        None => None,
        Some(t) => Some(timestamp(t, format)?),
    };
    if let Some(extra) = fields.next() {
        return Err(format!("`{extra}` after the timestamp"));
    }

    let (family, ty, suffix) = family_of(name, types);
    for (k, v) in target {
        if let Some(scraped) = labels.remove(*k) {
            labels.insert(format!("exported_{k}"), scraped);
        }
        labels.insert((*k).to_string(), (*v).to_string());
    }
    let labels = labels.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    Ok(PromSample {
        name: name.to_string(),
        family,
        ty,
        suffix,
        labels,
        value,
        ts,
    })
}

/// The family a sample name belongs to: itself if typed, else itself less a known suffix if
/// that is typed, else itself, untyped.
fn family_of(
    name: &str,
    types: &BTreeMap<String, FamilyType>,
) -> (String, FamilyType, &'static str) {
    for suffix in SUFFIXES {
        if let Some(base) = name.strip_suffix(suffix) {
            if let Some(ty) = types.get(base) {
                return (base.to_string(), *ty, suffix);
            }
        }
    }
    match types.get(name) {
        Some(ty) => (name.to_string(), *ty, ""),
        None => (name.to_string(), FamilyType::Unknown, ""),
    }
}

/// A sample's own timestamp: integer ms in text 0.0.4; seconds, possibly fractional, in
/// OpenMetrics, read exactly to the nanosecond.
fn timestamp(t: &str, format: Format) -> Result<(u64, Option<u32>), String> {
    let bad = || format!("`{t}` is not a timestamp");
    match format {
        Format::Text => {
            let ms: i64 = t.parse().map_err(|_| bad())?;
            let ms = u64::try_from(ms).map_err(|_| {
                format!(
                    "`{t}` is before 1970: an instant here is unsigned epoch ms, as `observed` is"
                )
            })?;
            Ok((ms, None))
        }
        Format::OpenMetrics => {
            let d = Decimal::parse_token(t).map_err(|_| bad())?;
            if d.mantissa() < 0 {
                return Err(bad());
            }
            // The instant in nanoseconds, exactly; a digit below the nanosecond is refused
            // rather than rounded, since rounding would be a choice the source did not make.
            let e = i32::from(d.exponent()) + 9;
            let mut ns = d.mantissa();
            if e >= 0 {
                for _ in 0..e {
                    ns = ns.checked_mul(10).ok_or_else(bad)?;
                }
            } else {
                for _ in 0..-e {
                    if ns % 10 != 0 {
                        return Err(format!("`{t}` is finer than a nanosecond"));
                    }
                    ns /= 10;
                }
            }
            let ms = u64::try_from(ns / 1_000_000).map_err(|_| bad())?;
            let sub = (ns % 1_000_000) as u32;
            Ok((ms, (sub != 0).then_some(sub)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"# HELP http_requests_total The total number of HTTP requests.
# TYPE http_requests_total counter
http_requests_total{method="post",code="200"} 1027 1395066363000
http_requests_total{method="post",code="400"}    3 1395066363000

# Escaping in label values:
msdos_file_access_time_seconds{path="C:\\DIR\\FILE.TXT",error="Cannot find file:\n\"FILE.TXT\""} 1.458255915e9

# Minimalistic line:
metric_without_timestamp_and_labels 12.47

# TYPE http_request_duration_seconds histogram
http_request_duration_seconds_bucket{le="0.05"} 24054
http_request_duration_seconds_bucket{le="+Inf"} 144320
http_request_duration_seconds_sum 53423
http_request_duration_seconds_count 144320

# TYPE rpc_duration_seconds summary
rpc_duration_seconds{quantile="0.5"} 4773
rpc_duration_seconds_count 2693
something_weird{problem="division by zero"} +Inf
"#;

    const OM: &str = "# TYPE acme_http_router_request_seconds summary
# UNIT acme_http_router_request_seconds seconds
# HELP acme_http_router_request_seconds Latency though all of ACME's HTTP request router.
acme_http_router_request_seconds_sum{path=\"/api/v1\",method=\"GET\"} 9036.32
acme_http_router_request_seconds_count{path=\"/api/v1\",method=\"GET\"} 807283.0
acme_http_router_request_seconds_created{path=\"/api/v1\",method=\"GET\"} 1605281325.0
# TYPE go_goroutines gauge
go_goroutines 69 1605281325.123456789
# TYPE foo counter
foo_total 17.0 1520879607.789 # {trace_id=\"KOO5S4vxi0o\"} 0.67
# EOF
";

    #[test]
    fn the_text_format_examples_read_exactly() {
        let s = read(TEXT, &[]).unwrap();
        assert_eq!(s.format, Format::Text);
        assert_eq!(s.samples.len(), 11);
        let first = &s.samples[0];
        assert_eq!(
            first.key(),
            r#"http_requests_total{code="200",method="post"}"#,
            "labels sorted by name"
        );
        assert_eq!(
            (first.family.as_str(), first.ty),
            ("http_requests_total", FamilyType::Counter)
        );
        assert_eq!(first.ts, Some((1_395_066_363_000, None)));
        assert!(first.is_monotonic());

        let dos = &s.samples[2];
        assert_eq!(dos.labels[1].1, "C:\\DIR\\FILE.TXT");
        assert_eq!(dos.labels[0].1, "Cannot find file:\n\"FILE.TXT\"");
        assert_eq!(
            dos.key(),
            r#"msdos_file_access_time_seconds{error="Cannot find file:\n\"FILE.TXT\"",path="C:\\DIR\\FILE.TXT"}"#,
            "the key escapes back"
        );
        assert_eq!(dos.value, Some(Decimal::parse_token("1458255915").unwrap()));
        assert_eq!(
            s.samples[3].value,
            Some(Decimal::parse_token("12.47").unwrap())
        );

        let bucket = &s.samples[4];
        assert_eq!(
            (bucket.family.as_str(), bucket.suffix),
            ("http_request_duration_seconds", "_bucket")
        );
        assert!(bucket.is_monotonic());
        assert!(!s.samples[6].is_monotonic(), "_sum may fall");
        assert_eq!(s.samples[8].suffix, "", "a summary's quantile sample");
        assert!(s.samples[9].is_monotonic());

        assert_eq!(s.samples[10].value, None, "+Inf is not a value");
        assert_eq!(s.not_values, 1);
        // The format's own example ends `+Inf -3982045`: before 1970, which no instant here is.
        let e = read("x 1 -5", &[]).unwrap_err().to_string();
        assert!(e.contains("before 1970"), "{e}");
    }

    #[test]
    fn openmetrics_reads_seconds_exactly_and_skips_exemplars() {
        let s = read(OM, &[]).unwrap();
        assert_eq!(s.format, Format::OpenMetrics);
        assert_eq!(s.samples.len(), 5);
        assert_eq!(s.samples[2].suffix, "_created");
        assert!(!s.samples[2].is_monotonic());
        assert!(s.samples[1].is_monotonic());
        assert_eq!(s.samples[3].ts, Some((1_605_281_325_123, Some(456_789))));
        let foo = &s.samples[4];
        assert_eq!((foo.family.as_str(), foo.suffix), ("foo", "_total"));
        assert_eq!(foo.ts, Some((1_520_879_607_789, None)));
        assert_eq!(foo.value, Some(Decimal::from_i64(17)));
        assert_eq!(s.units["acme_http_router_request_seconds"], "seconds");
        assert!(s.help["acme_http_router_request_seconds"].starts_with("Latency"));

        assert!(read("x 1\n# EOF\nx 2\n", &[]).is_err(), "after EOF");
        assert!(read("x 1\n\n# EOF\n", &[]).is_err(), "an empty line");
        assert!(
            read("x 1 1.0000000001\n# EOF\n", &[]).is_err(),
            "below a nanosecond"
        );
    }

    #[test]
    fn target_labels_join_identity_and_win() {
        let s = read(
            r#"up{job="scraped",x=""} 1"#,
            &[("job", "node"), ("instance", "h:9100")],
        )
        .unwrap();
        assert_eq!(
            s.samples[0].key(),
            r#"up{exported_job="scraped",instance="h:9100",job="node"}"#,
            "the empty `x` is absent"
        );
    }

    #[test]
    fn malformed_lines_are_refused_with_their_number() {
        for bad in [
            "1abc 2",
            "x{a=1} 2",
            "x{a=\"1\" b=\"2\"} 3",
            "x{a=\"1\",a=\"2\"} 3",
            "x{a=\"\\t\"} 1",
            "x",
            "x one",
            "x 1 2 3",
            "# TYPE x histogramme\nx 1",
            "# TYPE x gauge\n# TYPE x gauge\nx 1",
        ] {
            let e = read(bad, &[]).unwrap_err().to_string();
            assert!(e.starts_with("line "), "{bad:?}: {e}");
        }
        assert!(
            read("x{a=\"1\",} 1", &[]).is_ok(),
            "a trailing comma is allowed"
        );
        assert_eq!(read("", &[]).unwrap().samples.len(), 0);
    }
}
