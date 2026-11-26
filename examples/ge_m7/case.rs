//! One RCAEval RE2 case, read from its files and anonymised (RFC SMYSL-10 §5).
//!
//! A case directory is `<system>/<service>_<fault>/<rep>`, holding `inject_time.txt`,
//! `simple_metrics.csv`, `logs.csv` and, for Online Boutique and Train Ticket, `traces.csv`.
//! Sock Shop has no traces (R1-3, read at M7-P1). The label is in the path and nowhere else the
//! agent can reach: every name the case's own data uses for a service is replaced by `svc-NN`,
//! with the variants a service is spelt by in its logs and traces (R1-7).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// One metric column: its samples as (unix seconds, raw text).
pub type Column = Vec<(u64, String)>;

/// A case as read, before anonymisation.
pub struct Raw {
    pub system: String,
    /// `<service>_<fault>/<rep>`, relative to the system directory.
    pub rel: String,
    pub root: String,
    pub fault: String,
    /// Unix seconds of the first metric sample: the case's second 0.
    pub t0: u64,
    /// Unix seconds.
    pub inject: u64,
    /// By column name (`<service>_<metric>`).
    pub metrics: BTreeMap<String, Column>,
    /// (unix ms, container, message), in time order.
    pub logs: Vec<(u64, String, String)>,
    /// (unix ms start, service, operation, duration µs); `None` when the system has no traces.
    pub spans: Option<Vec<(u64, String, String, u64)>>,
}

pub fn csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn read(dir: &Path, f: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(f)).ok()
}

/// A metric column's service and metric: the last `_` splits them (`ts-order-service_cpu`).
pub fn split_column(col: &str) -> Option<(&str, &str)> {
    col.rsplit_once('_')
}

impl Raw {
    /// Read the case at `system_dir/rel`.
    pub fn load(system_dir: &Path, rel: &str) -> Result<Raw, String> {
        let dir = system_dir.join(rel);
        let system = system_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let (label, _) = rel
            .split_once('/')
            .ok_or_else(|| format!("{rel}: not <service>_<fault>/<rep>"))?;
        let (root, fault) = label
            .rsplit_once('_')
            .ok_or_else(|| format!("{rel}: not <service>_<fault>"))?;
        let need = |f: &str| read(&dir, f).ok_or_else(|| format!("{}: no {f}", dir.display()));
        let inject: u64 = need("inject_time.txt")?
            .trim()
            .parse()
            .map_err(|e| format!("{}: inject_time.txt: {e}", dir.display()))?;
        let rows = csv(&need("simple_metrics.csv")?);
        let head = rows.first().ok_or("simple_metrics.csv is empty")?;
        let mut metrics: BTreeMap<String, Column> = BTreeMap::new();
        let mut t0 = u64::MAX;
        for r in &rows[1..] {
            let Some(ts) = r.first().and_then(|t| t.parse::<f64>().ok()) else {
                continue;
            };
            let ts = ts as u64;
            t0 = t0.min(ts);
            for (j, name) in head.iter().enumerate().skip(1) {
                if split_column(name).is_none() {
                    continue;
                }
                if let Some(v) = r.get(j).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                    metrics
                        .entry(name.clone())
                        .or_default()
                        .push((ts, v.to_string()));
                }
            }
        }
        if t0 == u64::MAX {
            return Err(format!("{}: no metric samples", dir.display()));
        }
        let logs_rows = csv(&need("logs.csv")?);
        let col = |rows: &[Vec<String>], n: &str| -> Result<usize, String> {
            rows.first()
                .and_then(|h| h.iter().position(|x| x == n))
                .ok_or_else(|| format!("{}: no column {n}", dir.display()))
        };
        let (ts, who, msg) = (
            col(&logs_rows, "timestamp")?,
            col(&logs_rows, "container_name")?,
            col(&logs_rows, "message")?,
        );
        let mut logs: Vec<(u64, String, String)> = logs_rows[1..]
            .iter()
            .filter_map(|r| {
                Some((
                    r.get(ts)?.parse::<u64>().ok()? / 1_000_000,
                    r.get(who)?.clone(),
                    r.get(msg)?.clone(),
                ))
            })
            .collect();
        logs.sort();
        let spans = match read(&dir, "traces.csv") {
            None => None,
            Some(text) => {
                let rows = csv(&text);
                let (st, svc, op, dur) = (
                    col(&rows, "startTimeMillis")?,
                    col(&rows, "serviceName")?,
                    col(&rows, "operationName")?,
                    col(&rows, "duration")?,
                );
                let mut v: Vec<(u64, String, String, u64)> = rows[1..]
                    .iter()
                    .filter_map(|r| {
                        Some((
                            r.get(st)?.parse().ok()?,
                            r.get(svc)?.clone(),
                            r.get(op)?.clone(),
                            r.get(dur)?.parse().ok()?,
                        ))
                    })
                    .collect();
                v.sort();
                Some(v)
            }
        };
        Ok(Raw {
            system,
            rel: rel.to_string(),
            root: root.to_string(),
            fault: fault.to_string(),
            t0,
            inject,
            metrics,
            logs,
            spans,
        })
    }

    /// The services the case's data names: metric columns' and log containers'. Trace service
    /// names are mapped onto these by their variants.
    pub fn services(&self) -> BTreeSet<String> {
        let mut s: BTreeSet<String> = self
            .metrics
            .keys()
            .filter_map(|c| split_column(c).map(|x| x.0.to_string()))
            .collect();
        s.extend(self.logs.iter().map(|l| l.1.clone()));
        s
    }
}

/// The spellings a service goes by (R1-7): its name; without Train Ticket's `ts-` and a
/// `-service` or `service` suffix; each without hyphens; and the bare name with `service` and
/// `-service` appended, as Online Boutique's traces spell `frontend`.
pub fn variants(name: &str) -> BTreeSet<String> {
    let n = name.to_ascii_lowercase();
    let mut out = BTreeSet::new();
    let a = n.strip_prefix("ts-").unwrap_or(&n).to_string();
    let b = a
        .strip_suffix("-service")
        .or_else(|| a.strip_suffix("service"))
        .unwrap_or(&a)
        .to_string();
    // The data's own forms of the bare name: `cart` for `carts`, `recommendations` for
    // `recommendation`, `catalog` for `catalogue` (the M7-P1 survey of RE2's logs).
    let mut bases = BTreeSet::from([b.clone()]);
    if let Some(x) = b.strip_suffix('s') {
        bases.insert(x.to_string());
    } else {
        bases.insert(format!("{b}s"));
    }
    if let Some(x) = b.strip_suffix("ue") {
        bases.insert(x.to_string());
    }
    let mut forms = vec![n.clone(), a.clone()];
    for x in &bases {
        forms.extend([x.clone(), format!("{x}service"), format!("{x}-service")]);
    }
    for v in forms {
        if v.is_empty() {
            continue;
        }
        out.insert(v.replace('-', ""));
        out.insert(v);
    }
    out.retain(|v| v.len() >= 3);
    out
}

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

pub fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// The renaming of one case's services.
pub struct Anon {
    /// Original service → `svc-NN`.
    pub to: BTreeMap<String, String>,
    /// `svc-NN` → original service.
    pub from: BTreeMap<String, String>,
    /// Lower-case variant → `svc-NN`, longest first when matched.
    variants: Vec<(String, String)>,
    /// Token → its renaming: logs repeat their tokens, and each miss scans every variant.
    memo: RefCell<HashMap<String, String>>,
}

impl Anon {
    /// A seeded permutation of `services`, the seed the case's path, so every arm of a case sees
    /// one renaming.
    pub fn new(services: &BTreeSet<String>, seed: &str) -> Anon {
        let mut order: Vec<&String> = services.iter().collect();
        let mut rng = Rng(fnv(seed));
        for i in (1..order.len()).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        let to: BTreeMap<String, String> = order
            .iter()
            .enumerate()
            .map(|(i, s)| ((*s).clone(), format!("svc-{:02}", i + 1)))
            .collect();
        let from = to.iter().map(|(a, b)| (b.clone(), a.clone())).collect();
        // A variant two services share names neither.
        let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (s, anon) in &to {
            for v in variants(s) {
                owners.entry(v).or_default().insert(anon.clone());
            }
        }
        let mut variants: Vec<(String, String)> = owners
            .into_iter()
            .filter(|(_, o)| o.len() == 1)
            .map(|(v, o)| (v, o.into_iter().next().expect("one")))
            .collect();
        variants.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Anon {
            to,
            from,
            variants,
            memo: RefCell::default(),
        }
    }

    /// The `svc-NN` a name in the data stands for: the service, or one of its variants.
    pub fn of(&self, name: &str) -> Option<&str> {
        if let Some(a) = self.to.get(name) {
            return Some(a);
        }
        let l = name.to_ascii_lowercase();
        self.variants
            .iter()
            .find(|(v, _)| *v == l)
            .map(|(_, a)| a.as_str())
    }

    /// `text` with every service spelling replaced: a whole token (letters, digits, `-`, `_`),
    /// a token that starts with a spelling and a `-` (a pod name), and, for spellings of six
    /// characters or more, any occurrence inside a token, case ignored (`OrderServiceImpl`).
    pub fn text(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut tok = String::new();
        let flush = |tok: &mut String, out: &mut String| {
            if !tok.is_empty() {
                let mut memo = self.memo.borrow_mut();
                match memo.get(tok.as_str()) {
                    Some(t) => out.push_str(t),
                    None => {
                        let t = self.token(tok);
                        out.push_str(&t);
                        memo.insert(std::mem::take(tok), t);
                    }
                }
                tok.clear();
            }
        };
        for c in text.chars() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                tok.push(c);
            } else {
                flush(&mut tok, &mut out);
                out.push(c);
            }
        }
        flush(&mut tok, &mut out);
        out
    }

    fn token(&self, tok: &str) -> String {
        let l = tok.to_ascii_lowercase();
        for (v, a) in &self.variants {
            if l == *v {
                return a.clone();
            }
            if l.starts_with(v.as_str()) && l.as_bytes().get(v.len()) == Some(&b'-') {
                return format!("{a}{}", &tok[v.len()..]);
            }
        }
        // Inside a token: long spellings only, so `user` does not eat `username`.
        let mut s = tok.to_string();
        for (v, a) in self.variants.iter().filter(|(v, _)| v.len() >= 6) {
            loop {
                let low = s.to_ascii_lowercase();
                match low.find(v.as_str()) {
                    Some(i) => s.replace_range(i..i + v.len(), a),
                    None => break,
                }
            }
        }
        // CamelCase: a segment that is a spelling, so `CartsController` and `setCurrency` lose
        // the name while `username` keeps it (the segment rule needs a case boundary).
        let segs = camel(&s);
        if segs.len() > 1 {
            let mut out = String::with_capacity(s.len());
            for seg in segs {
                let l = seg.to_ascii_lowercase();
                match self.variants.iter().find(|(v, _)| *v == l) {
                    Some((_, a)) => out.push_str(a),
                    None => out.push_str(seg),
                }
            }
            s = out;
        }
        s
    }

    /// The spellings of `service`, for the leakage check.
    pub fn spellings(&self, service: &str) -> Vec<String> {
        let a = self.to.get(service);
        self.variants
            .iter()
            .filter(|(_, x)| Some(x) == a)
            .map(|(v, _)| v.clone())
            .collect()
    }
}

/// A token's CamelCase segments: a boundary before an upper-case letter that follows a
/// lower-case letter or a digit.
pub fn camel(tok: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<char> = None;
    for (i, c) in tok.char_indices() {
        if c.is_ascii_uppercase()
            && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
        {
            out.push(&tok[start..i]);
            start = i;
        }
        prev = Some(c);
    }
    out.push(&tok[start..]);
    out
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|t| !t.is_empty())
}

/// Where a leak of `root`'s name is in `text`, if anywhere: a whole token or a CamelCase segment
/// equal to a spelling, or a spelling of six characters or more inside a token.
pub fn leak(text: &str, spellings: &[String]) -> Option<String> {
    for tok in tokens(text) {
        let low = tok.to_ascii_lowercase();
        for v in spellings {
            if low == *v
                || (v.len() >= 6 && low.contains(v.as_str()))
                || camel(tok).iter().any(|s| s.eq_ignore_ascii_case(v))
            {
                return Some(tok.to_string());
            }
        }
    }
    None
}

/// The tokens of `text` that hold `stem` anywhere, ignoring case: what the renaming leaves of a
/// root's name, reported apart from [`leak`] so the check does not rest on the renaming's rules.
pub fn residue<'a>(text: &'a str, stem: &str) -> Vec<&'a str> {
    tokens(text)
        .filter(|t| t.to_ascii_lowercase().contains(stem))
        .collect()
}

/// A service's bare stem: no `ts-`, no `service`, no plural `s`, no `ue`.
pub fn stem(service: &str) -> String {
    let n = service.to_ascii_lowercase();
    let a = n.strip_prefix("ts-").unwrap_or(&n);
    let b = a
        .strip_suffix("-service")
        .or_else(|| a.strip_suffix("service"))
        .unwrap_or(a);
    let b = b.strip_suffix('s').unwrap_or(b);
    b.strip_suffix("ue").unwrap_or(b).to_string()
}

/// The case directories under `system_dir`, as `<service>_<fault>/<rep>`, sorted.
pub fn list(system_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(system_dir) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        let name = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !d.is_dir() || name.rsplit_once('_').is_none() {
            continue;
        }
        let mut reps: Vec<String> = std::fs::read_dir(&d)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
                    .collect()
            })
            .unwrap_or_default();
        reps.sort();
        out.extend(reps.into_iter().map(|r| format!("{name}/{r}")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_cover_the_spellings_the_data_uses() {
        let v = variants("ts-order-service");
        for s in ["ts-order-service", "order-service", "orderservice", "order"] {
            assert!(v.contains(s), "{s}");
        }
        assert!(variants("frontend").contains("frontendservice"));
        assert!(variants("front-end").contains("frontend"));
    }

    #[test]
    fn renaming_catches_the_forms_sock_shop_and_online_boutique_use() {
        let s: BTreeSet<String> = [
            "carts",
            "carts-db",
            "catalogue",
            "recommendationservice",
            "user",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let a = Anon::new(&s, "y");
        let (c, cd, k, r, u) = (
            &a.to["carts"],
            &a.to["carts-db"],
            &a.to["catalogue"],
            &a.to["recommendationservice"],
            &a.to["user"],
        );
        let t = a.text("cart Carts CartResource CartsController carts-db catalog ListRecommendations username GetUsers");
        assert_eq!(
            t,
            format!("{c} {c} {c}Resource {c}Controller {cd} {k} List{r} username Get{u}")
        );
        assert_eq!(leak(&t, &a.spellings("carts")), None);
        assert_eq!(
            leak("a CartResource", &a.spellings("carts")),
            Some("CartResource".into())
        );
        assert_eq!(residue(&t, &stem("user")), vec!["username"]);
    }

    #[test]
    fn renaming_replaces_tokens_pods_and_class_names() {
        let s: BTreeSet<String> = ["ts-order-service", "ts-order-other-service", "carts"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = Anon::new(&s, "x");
        let o = a.to["ts-order-service"].clone();
        let oo = a.to["ts-order-other-service"].clone();
        let t = a.text(
            "GET /api/v1/orderservice/order from ts-order-service-7d9f-abc, \
             OrderServiceImpl; ts-order-other-service ok",
        );
        assert_eq!(
            t,
            format!("GET /api/v1/{o}/{o} from {o}-7d9f-abc, {o}Impl; {oo} ok")
        );
        assert_eq!(leak(&t, &a.spellings("ts-order-service")), None);
        assert!(leak("hello orderservice", &a.spellings("ts-order-service")).is_some());
    }
}
