# Pool saturation is the leading cause; the clean canary is unexplained

*brief · profile analyst*
*for the person who has to check this*

## bottom-line

[derived] Pool saturation is the leading cause

Connection wait time tracks the latency curve within the noise floor.

> **contested** — k/pool-vs-canary: contested, 1 position(s) on record

## support

[measured] Consequently, p95 request latency rose from 180ms to 410ms after the 4.2 rollout

Measured on the eu-west shard over one-minute windows, 14:00-16:00 UTC.

Sampled at 10s resolution; the pre-rollout baseline is the trailing 7-day median.

*metric: p95_request_seconds*

*model:vendor/m (computed)*

## risk

[speculative] The canary shard was clean throughout

> **contested** — k/pool-vs-canary: contested, 1 position(s) on record

## ask

[inferred] Roll the eu-west shard back to 4.1 and re-measure

---

**Open contentions:** k/pool-vs-canary
