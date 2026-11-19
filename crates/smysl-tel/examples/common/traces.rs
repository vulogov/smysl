//! Simulated traces for GE-M2 and GE-M5 (RFC SMYSL-3 §26): a social-network service graph after
//! DeathStarBench's, in OTLP's span model.
//!
//! - **Eleven instrumented services**, one per host. A call between two of them is a client span
//!   on the caller, child of the caller's server span, and a server span on the callee, child of
//!   the client span.
//! - **memcached, redis and MongoDB** are not instrumented: a call to one is a client span on the
//!   caller only, as a driver records it.
//! - **Stages.** A handler makes its calls in stages, parallel within one and sequential across
//!   them. A run of cache reads is a stage each.
//! - **Latency** is log-normal, with medians of the published order: a network hop 0.08 ms, a
//!   cache call 0.12 ms, a redis call 0.15 ms, MongoDB 1.2 ms, a handler's own work 0.05–0.4 ms.
//! - **Clocks.** Every span is stamped by its host's clock: the true time, plus the host's
//!   residual offset (uniform in ±0.25 ms), plus any injected skew.
//!
//! All of it is a function of the seed (splitmix64), so a run is reproducible.

#![allow(dead_code)]

use smysl_tel::readers::otlp::AnyValue;
use smysl_tel::readers::otlp_traces::Span;

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

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u = self.unit().max(1e-12);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }

    /// Log-normal with this median (ns) and log-sd.
    pub fn lognormal(&mut self, median_ns: f64, sigma: f64) -> u64 {
        (median_ns * (sigma * self.normal()).exp()).max(1.0) as u64
    }
}

/// The instrumented services, one per host.
pub const SERVICES: [&str; 11] = [
    "nginx-web",
    "compose-post",
    "text",
    "url-shorten",
    "user-mention",
    "unique-id",
    "media",
    "user",
    "post-storage",
    "home-timeline",
    "social-graph",
];

const NGINX: usize = 0;
const COMPOSE: usize = 1;
const TEXT: usize = 2;
const URL: usize = 3;
const MENTION: usize = 4;
const UNIQUE: usize = 5;
const MEDIA: usize = 6;
const USER: usize = 7;
const STORAGE: usize = 8;
const HOME: usize = 9;
const GRAPH: usize = 10;

/// An uninstrumented store.
#[derive(Clone, Copy)]
enum Store {
    Memcached,
    Redis,
    Mongo,
}

impl Store {
    fn name(self) -> &'static str {
        match self {
            Store::Memcached => "memcached get",
            Store::Redis => "redis",
            Store::Mongo => "mongodb find",
        }
    }

    fn median_ns(self) -> f64 {
        match self {
            Store::Memcached => 120_000.0,
            Store::Redis => 150_000.0,
            Store::Mongo => 1_200_000.0,
        }
    }
}

/// One call a handler makes.
#[derive(Clone, Copy)]
enum Call {
    Service(usize),
    Store(Store),
}

const HOP_NS: f64 = 80_000.0;

/// One span as simulated: true times, and the host whose clock stamps it.
#[derive(Clone, Debug)]
pub struct SimSpan {
    pub id: [u8; 8],
    pub parent: Option<[u8; 8]>,
    pub host: usize,
    pub name: String,
    /// 2 server, 3 client.
    pub kind: u32,
    pub start: u64,
    pub end: u64,
}

/// One trace.
pub struct SimTrace {
    pub id: [u8; 16],
    pub spans: Vec<SimSpan>,
}

struct Gen<'a> {
    r: &'a mut Rng,
    spans: Vec<SimSpan>,
}

impl Gen<'_> {
    fn id(&mut self) -> [u8; 8] {
        let mut b = self.r.next().to_be_bytes();
        b[0] |= 1;
        b
    }

    /// The stages `svc`'s handler runs: each a list of calls made together.
    fn stages(&mut self, svc: usize) -> Vec<Vec<Call>> {
        use Call::{Service as S, Store as D};
        let k = |r: &mut Rng, lo: u64, hi: u64| lo + r.below(hi - lo + 1);
        match svc {
            NGINX => vec![vec![S(if self.r.unit() < 0.1 { COMPOSE } else { HOME })]],
            COMPOSE => vec![
                vec![S(TEXT), S(UNIQUE), S(MEDIA), S(USER)],
                vec![S(STORAGE)],
                vec![S(HOME)],
            ],
            TEXT => vec![vec![S(URL), S(MENTION)], vec![D(Store::Mongo)]],
            URL => vec![vec![D(Store::Memcached)], vec![D(Store::Mongo)]],
            MENTION => (0..k(self.r, 1, 4))
                .map(|_| vec![D(Store::Memcached)])
                .collect(),
            UNIQUE => vec![],
            MEDIA => vec![vec![D(Store::Memcached)]],
            USER => vec![vec![D(Store::Memcached)]],
            STORAGE => {
                let mut v: Vec<Vec<Call>> = (0..k(self.r, 1, 5))
                    .map(|_| vec![D(Store::Memcached)])
                    .collect();
                if self.r.unit() < 0.3 {
                    v.push(vec![D(Store::Mongo)]);
                }
                v
            }
            HOME => {
                let mut v = vec![vec![S(GRAPH)], vec![D(Store::Redis)]];
                if self.r.unit() < 0.7 {
                    v.push(vec![S(STORAGE)]);
                }
                v
            }
            GRAPH => (0..k(self.r, 1, 3))
                .map(|_| vec![D(Store::Redis)])
                .collect(),
            _ => vec![],
        }
    }

    /// The server span of `svc` handling a request that arrives at `t`, under `parent`; returns
    /// its end.
    fn serve(&mut self, svc: usize, t: u64, parent: Option<[u8; 8]>) -> u64 {
        let id = self.id();
        let idx = self.spans.len();
        self.spans.push(SimSpan {
            id,
            parent,
            host: svc,
            name: format!("{} handle", SERVICES[svc]),
            kind: 2,
            start: t,
            end: t,
        });
        let mut now = t + self.r.lognormal(50_000.0, 0.5);
        for stage in self.stages(svc) {
            let mut done = now;
            for call in stage {
                let at = now + self.r.lognormal(10_000.0, 0.5);
                let end = self.call(svc, id, call, at);
                done = done.max(end);
            }
            now = done;
        }
        now += self.r.lognormal(150_000.0, 0.7);
        self.spans[idx].end = now;
        now
    }

    /// A call `from` makes at `t`, under the server span `parent`; returns when its client span
    /// ends.
    fn call(&mut self, from: usize, parent: [u8; 8], call: Call, t: u64) -> u64 {
        let id = self.id();
        let idx = self.spans.len();
        let name = match call {
            Call::Service(to) => format!("{} call", SERVICES[to]),
            Call::Store(s) => s.name().to_string(),
        };
        self.spans.push(SimSpan {
            id,
            parent: Some(parent),
            host: from,
            name,
            kind: 3,
            start: t,
            end: t,
        });
        let end = match call {
            Call::Store(s) => t + self.r.lognormal(s.median_ns(), 0.5),
            Call::Service(to) => {
                let arrive = t + self.r.lognormal(HOP_NS, 0.4);
                let served = self.serve(to, arrive, Some(id));
                served + self.r.lognormal(HOP_NS, 0.4)
            }
        };
        self.spans[idx].end = end;
        end
    }
}

/// One trace, its request arriving at nginx at `t` (true ns).
pub fn trace(r: &mut Rng, t: u64) -> SimTrace {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&r.next().to_be_bytes());
    id[8..].copy_from_slice(&(r.next() | 1).to_be_bytes());
    let mut g = Gen {
        r,
        spans: Vec::new(),
    };
    g.serve(NGINX, t, None);
    SimTrace { id, spans: g.spans }
}

/// `n` traces arriving at `rate` a second from `t0` (Poisson).
pub fn traces(r: &mut Rng, t0: u64, n: usize, rate: f64) -> Vec<SimTrace> {
    let mut t = t0 as f64;
    (0..n)
        .map(|_| {
            t += -(1.0 - r.unit()).ln() / rate * 1e9;
            trace(r, t as u64)
        })
        .collect()
}

/// Each host's residual clock offset, ns: uniform in ±0.25 ms.
pub fn offsets(r: &mut Rng) -> Vec<i64> {
    (0..SERVICES.len())
        .map(|_| (r.unit() * 500_000.0) as i64 - 250_000)
        .collect()
}

/// The OTLP spans of `traces`, stamped by each host's clock: `offsets[host] + skew[host]`.
pub fn otlp(traces: &[SimTrace], offsets: &[i64], skew: &[i64]) -> Vec<Span> {
    let at = |t: u64, h: usize| (t as i64 + offsets[h] + skew[h]) as u64;
    let mut out = Vec::new();
    for tr in traces {
        for s in &tr.spans {
            out.push(Span {
                resource: vec![
                    (
                        "service.name".into(),
                        AnyValue::Str(SERVICES[s.host].into()),
                    ),
                    (
                        "service.instance.id".into(),
                        AnyValue::Str(format!("h{}", s.host)),
                    ),
                ],
                scope_name: "sim".into(),
                scope_version: "1".into(),
                trace_id: tr.id,
                span_id: s.id,
                parent: s.parent,
                name: s.name.clone(),
                kind: s.kind,
                start_ns: at(s.start, s.host),
                end_ns: at(s.end, s.host),
                status: 0,
                status_message: String::new(),
                attributes: Vec::new(),
            });
        }
    }
    out
}

/// 2026-01-16T10:00:00Z, in ns.
pub const T0_NS: u64 = 1_768_557_600_000_000_000;
