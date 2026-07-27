//! The HTTP wrapper (§21).
//!
//! One place where transport and status errors become [`ProviderError`], so five mappers
//! do not each invent their own mapping and disagree about what a 429 means.
//!
//! `RateLimited` retries with exponential backoff and full jitter, capped at three
//! attempts. Retries are counted in [`Usage::retries`] and never appear in provenance: a
//! retry is not a distinct model call for recipe purposes (§21.4).

use std::time::Duration;

use smysl_core::error::ProviderError;

/// Attempts, not retries: three attempts is two retries (§21.4).
pub const MAX_ATTEMPTS: u32 = 3;

/// The base of the exponential backoff.
pub const BACKOFF_BASE: Duration = Duration::from_millis(250);

/// A completed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// How many attempts were retried before this one succeeded.
    pub retries: u32,
}

/// Full jitter over an exponential backoff: `rand(0, base * 2^attempt)`.
///
/// Full jitter rather than a fixed multiplier, because several clients backing off in
/// lockstep produce another thundering herd at every step - which is the failure the
/// backoff exists to avoid.
pub fn backoff(attempt: u32, jitter: f64) -> Duration {
    let capped = attempt.min(10);
    let ceiling = BACKOFF_BASE.saturating_mul(1u32 << capped);
    ceiling.mul_f64(jitter.clamp(0.0, 1.0))
}

/// Whether an error is worth trying again.
///
/// `RateLimited` is: the server said "later". Nothing else is - `Unauthorized` will fail
/// identically the second time, and retrying `Malformed` would hide a mapper bug behind a
/// delay.
pub const fn is_retryable(e: &ProviderError) -> bool {
    matches!(e, ProviderError::RateLimited { .. })
}

/// Map an HTTP status onto a [`ProviderError`].
pub fn status_error(status: u16, body: &str, retry_after: Option<Duration>) -> ProviderError {
    match status {
        401 | 403 => ProviderError::Unauthorized,
        429 => ProviderError::RateLimited { retry_after },
        // 413 and 422 are how the common endpoints say "too long"; the numbers the caller
        // needs are in the body and the mapper fills them in if it can parse them.
        413 => ProviderError::ContextExceeded {
            limit: 0,
            requested: 0,
        },
        s => ProviderError::Upstream(s, truncate(body)),
    }
}

/// Bodies are untrusted input and error messages reach logs, so a multi-megabyte HTML
/// error page does not become a multi-megabyte diagnostic.
fn truncate(body: &str) -> String {
    const LIMIT: usize = 512;
    if body.len() <= LIMIT {
        return body.trim().to_string();
    }
    let mut end = LIMIT;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", body[..end].trim())
}

/// Parse a `Retry-After` header, which may be seconds or an HTTP date. Only the seconds
/// form is honoured; a date would need a calendar and the difference is not worth one.
pub fn parse_retry_after(v: &str) -> Option<Duration> {
    v.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(feature = "http-client")]
mod client {
    use super::*;

    /// POST a JSON body, retrying on rate limiting with backoff and full jitter.
    ///
    /// **A status code is data, not an error.** Only a transport failure returns `Err`;
    /// every HTTP response comes back as `Ok`, body included. Responsibility 5 of the
    /// mapper contract is error mapping, and a mapper cannot map what this layer already
    /// threw away - Ollama answers a missing model with 404 *and* an explanatory body, and
    /// the body is the half that matters.
    ///
    /// `sleep` and `jitter` are injected so the retry policy is testable without waiting:
    /// a backoff test that actually slept would be a slow test of the clock rather than a
    /// fast test of the policy.
    pub fn post_json(
        url: &str,
        headers: &[(&str, String)],
        body: &str,
        timeout: Duration,
        mut sleep: impl FnMut(Duration),
        mut jitter: impl FnMut() -> f64,
    ) -> Result<HttpResponse, ProviderError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .user_agent(concat!("smysl/", env!("CARGO_PKG_VERSION")))
            .build();

        let mut retries = 0u32;
        for attempt in 0..MAX_ATTEMPTS {
            let mut req = agent.post(url).set("content-type", "application/json");
            for (k, v) in headers {
                req = req.set(k, v);
            }
            match req.send_string(body) {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.into_string().map_err(|e| {
                        ProviderError::Malformed(format!("unreadable response body: {e}"))
                    })?;
                    return Ok(HttpResponse {
                        status,
                        body: text,
                        retries,
                    });
                }
                Err(ureq::Error::Status(status, resp)) => {
                    let retry_after = resp
                        .header("retry-after")
                        .and_then(super::parse_retry_after);
                    let text = resp.into_string().unwrap_or_default();
                    // Rate limiting is the one status this layer acts on, because acting on
                    // it means waiting rather than deciding what it means.
                    if status != 429 || attempt + 1 == MAX_ATTEMPTS {
                        return Ok(HttpResponse {
                            status,
                            body: text,
                            retries,
                        });
                    }
                    retries += 1;
                    sleep(retry_after.unwrap_or_else(|| backoff(attempt, jitter())));
                }
                // Every transport failure is `Unreachable`: a DNS failure, a refused
                // connection and a timeout are the same thing to a caller deciding whether
                // to fall back.
                Err(ureq::Error::Transport(_)) => return Err(ProviderError::Unreachable),
            }
        }
        Err(ProviderError::Unreachable)
    }

    /// GET, for probes. Probes never retry: `providers --probe` reports what it found, and
    /// a probe that waited three seconds to say "unreachable" would be worse at its job.
    pub fn get(url: &str, timeout: Duration) -> Result<HttpResponse, ProviderError> {
        get_with(url, &[], timeout)
    }

    /// GET with headers, for a probe that needs a credential to be told anything.
    pub fn get_with(
        url: &str,
        headers: &[(&str, String)],
        timeout: Duration,
    ) -> Result<HttpResponse, ProviderError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(timeout)
            .user_agent(concat!("smysl/", env!("CARGO_PKG_VERSION")))
            .build();
        let mut req = agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.call() {
            Ok(resp) => {
                let status = resp.status();
                let body = resp
                    .into_string()
                    .map_err(|e| ProviderError::Malformed(e.to_string()))?;
                Ok(HttpResponse {
                    status,
                    body,
                    retries: 0,
                })
            }
            Err(ureq::Error::Status(status, resp)) => Ok(HttpResponse {
                status,
                body: resp.into_string().unwrap_or_default(),
                retries: 0,
            }),
            Err(ureq::Error::Transport(_)) => Err(ProviderError::Unreachable),
        }
    }
}

#[cfg(feature = "http-client")]
pub use client::{get, get_with, post_json};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rate_limiting_is_retryable() {
        assert!(is_retryable(&ProviderError::RateLimited {
            retry_after: None
        }));
        for e in [
            ProviderError::Unreachable,
            ProviderError::Unauthorized,
            ProviderError::OfflineViolation,
            ProviderError::StructuredUnsupported,
            ProviderError::Malformed("x".into()),
            ProviderError::ContextExceeded {
                limit: 1,
                requested: 2,
            },
            ProviderError::Upstream(500, "x".into()),
        ] {
            assert!(!is_retryable(&e), "{e} should not be retried");
        }
    }

    #[test]
    fn statuses_map_onto_the_error_vocabulary() {
        assert_eq!(status_error(401, "", None), ProviderError::Unauthorized);
        assert_eq!(status_error(403, "", None), ProviderError::Unauthorized);
        assert!(matches!(
            status_error(429, "", None),
            ProviderError::RateLimited { .. }
        ));
        assert!(matches!(
            status_error(413, "", None),
            ProviderError::ContextExceeded { .. }
        ));
        assert!(matches!(
            status_error(500, "boom", None),
            ProviderError::Upstream(500, _)
        ));
    }

    #[test]
    fn a_retry_after_header_reaches_the_error() {
        let e = status_error(429, "", parse_retry_after("30"));
        assert_eq!(
            e,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
        );
    }

    /// Only the seconds form is honoured; a date would need a calendar.
    #[test]
    fn an_http_date_retry_after_is_ignored_rather_than_guessed() {
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after(" 12 "), Some(Duration::from_secs(12)));
    }

    #[test]
    fn backoff_grows_exponentially() {
        let full = |a| backoff(a, 1.0);
        assert_eq!(full(0), BACKOFF_BASE);
        assert_eq!(full(1), BACKOFF_BASE * 2);
        assert_eq!(full(2), BACKOFF_BASE * 4);
        assert!(full(3) > full(2));
    }

    /// Full jitter, not a fixed multiplier: several clients backing off in lockstep would
    /// produce another herd at every step.
    #[test]
    fn jitter_spans_the_whole_interval() {
        assert_eq!(backoff(3, 0.0), Duration::ZERO);
        assert_eq!(backoff(3, 1.0), BACKOFF_BASE * 8);
        assert_eq!(backoff(3, 0.5), BACKOFF_BASE * 4);
    }

    #[test]
    fn jitter_outside_the_unit_interval_is_clamped_rather_than_trusted() {
        assert_eq!(backoff(2, 5.0), backoff(2, 1.0));
        assert_eq!(backoff(2, -1.0), Duration::ZERO);
    }

    #[test]
    fn backoff_does_not_overflow_on_a_silly_attempt_count() {
        assert!(backoff(u32::MAX, 1.0) > Duration::ZERO);
    }

    /// Error messages reach logs, and a provider's error page is untrusted input.
    #[test]
    fn a_huge_error_body_is_truncated() {
        let e = status_error(500, &"x".repeat(10_000), None);
        match e {
            ProviderError::Upstream(_, msg) => {
                assert!(msg.len() < 600, "{}", msg.len());
                assert!(msg.ends_with('…'));
            }
            other => panic!("{other}"),
        }
    }

    #[test]
    fn truncation_lands_on_a_character_boundary() {
        let body = "é".repeat(1000);
        let e = status_error(500, &body, None);
        match e {
            ProviderError::Upstream(_, msg) => assert!(msg.chars().count() > 0),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn three_attempts_is_two_retries() {
        assert_eq!(MAX_ATTEMPTS, 3);
    }
}
