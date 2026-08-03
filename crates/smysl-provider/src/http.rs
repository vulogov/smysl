//! The HTTP wrapper (§21).
//!
//! One place where transport and status errors become [`ProviderError`], so five mappers
//! do not each invent their own mapping and disagree about what a 429 means.
//!
//! `RateLimited` retries with exponential backoff and full jitter, capped at three
//! attempts. Retries are counted in [`Usage::retries`](crate::Usage::retries) and never
//! appear in provenance: a
//! retry is not a distinct model call for recipe purposes (§21.4).
//!
//! **Backpressure is a class, not a status code.** See [`is_backpressure`]: three numbers
//! mean "the server said later", and telling them apart matters to nobody downstream.

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

/// The statuses that mean "the server said later", as opposed to "the request was wrong".
///
/// - **429** — too many requests, the classic form.
/// - **503** — unavailable. Google returns it for "this model is currently experiencing
///   high demand", which is backpressure wearing a server-error number, and it is what a
///   free-tier Gemini key sees most often.
/// - **529** — Anthropic's overloaded, which is 503 under a number of its own.
///
/// **500 is deliberately absent.** An internal server error is a bug on the far side, and
/// waiting 250ms does not fix a bug; retrying it would turn one failure into three.
///
/// Retried rather than distinguished, because acting on backpressure means waiting, and
/// waiting is the same wait whichever of the three arrived.
pub const fn is_backpressure(status: u16) -> bool {
    matches!(status, 429 | 503 | 529)
}

/// Map an HTTP status onto a [`ProviderError`].
pub fn status_error(status: u16, body: &str, retry_after: Option<Duration>) -> ProviderError {
    match status {
        401 | 403 => ProviderError::Unauthorized,
        s if is_backpressure(s) => ProviderError::RateLimited { retry_after },
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
                    // Backpressure is the one class this layer acts on, because acting on it
                    // means waiting rather than deciding what it means. The last attempt
                    // returns the response instead of sleeping: a wait nobody is going to
                    // use is just a slower failure.
                    if !is_backpressure(status) || attempt + 1 == MAX_ATTEMPTS {
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

    /// Three numbers, one meaning. 503 is Google's "high demand" and 529 is Anthropic's
    /// overloaded; both are the server saying later, not the request being wrong.
    #[test]
    fn backpressure_is_a_class_of_three_statuses() {
        for s in [429, 503, 529] {
            assert!(is_backpressure(s), "{s} is backpressure");
            assert!(
                is_retryable(&status_error(s, "", None)),
                "{s} must reach the retry loop"
            );
        }
    }

    /// Waiting 250ms does not fix a bug on the far side, and retrying would turn one
    /// failure into three.
    #[test]
    fn a_server_fault_is_not_backpressure() {
        for s in [400, 404, 413, 500, 502, 504] {
            assert!(!is_backpressure(s), "{s} must not be retried");
        }
        assert!(!is_retryable(&status_error(500, "boom", None)));
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

    /// The retry loop against a real socket. `is_backpressure` is a predicate anyone can
    /// assert; that `post_json` *acts* on it is the claim worth testing, and it needs a
    /// server that answers.
    #[cfg(feature = "http-client")]
    mod loop_over_a_socket {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        /// A server that answers each connection with the next status in the list, and goes
        /// on answering with the last one. Loopback and ephemeral: no fixture, no port to
        /// collide on.
        ///
        /// It keeps serving rather than stopping at the end of the list because a server
        /// that closes its listener turns any further connection into `ECONNREFUSED`, which
        /// arrives as `Unreachable` and fails the test for a reason that has nothing to do
        /// with the retry policy. That made this suite flaky: the statuses under test were
        /// always delivered, but a connection the client opened afterwards could land on a
        /// closed port depending on scheduling.
        fn serve(statuses: Vec<u16>) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                let last = *statuses.last().unwrap_or(&200);
                let mut queue = statuses.into_iter();
                loop {
                    let status = queue.next().unwrap_or(last);
                    let Ok((mut sock, _)) = listener.accept() else {
                        return;
                    };
                    // Read the *whole* request before answering. A single `read` can return
                    // just the headers, leaving the client still writing its body into a
                    // socket that is about to close - which arrives as a connection reset
                    // and fails the test as `Unreachable`, for reasons having nothing to do
                    // with the retry policy. That was this suite's flake: roughly one run in
                    // ten, depending on how the request was segmented.
                    let mut req = Vec::new();
                    let mut buf = [0u8; 1024];
                    let want = loop {
                        let head_end = req.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                        if let Some(head_end) = head_end {
                            let head = String::from_utf8_lossy(&req[..head_end]).to_lowercase();
                            let len: usize = head
                                .split("content-length:")
                                .nth(1)
                                .and_then(|t| t.split("\r\n").next())
                                .and_then(|t| t.trim().parse().ok())
                                .unwrap_or(0);
                            if req.len() >= head_end + len {
                                break true;
                            }
                        }
                        match sock.read(&mut buf) {
                            Ok(0) | Err(_) => break false,
                            Ok(n) => req.extend_from_slice(&buf[..n]),
                        }
                    };
                    if !want {
                        continue;
                    }

                    let body = format!(r#"{{"status":{status}}}"#);
                    let _ = sock.write_all(
                        format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    );
                    let _ = sock.flush();
                }
            });
            format!("http://{addr}/v1/x")
        }

        fn post(url: &str) -> HttpResponse {
            let mut slept = Vec::new();
            let r = post_json(
                url,
                &[],
                "{}",
                Duration::from_secs(5),
                |d| slept.push(d),
                || 0.0, // no real waiting: the policy is under test, not the clock
            )
            .unwrap();
            assert_eq!(slept.len(), r.retries as usize, "one sleep per retry");
            r
        }

        /// The change this test exists for: 503 is waited out rather than surfaced.
        #[test]
        fn a_503_is_retried_and_the_recovery_is_returned() {
            let r = post(&serve(vec![503, 503, 200]));
            assert_eq!(r.status, 200);
            assert_eq!(r.retries, 2, "two 503s were waited out");
        }

        #[test]
        fn a_429_is_retried_as_it_always_was() {
            let r = post(&serve(vec![429, 200]));
            assert_eq!(r.status, 200);
            assert_eq!(r.retries, 1);
        }

        /// Anthropic's overloaded, which is 503 under a number of its own.
        #[test]
        fn a_529_is_retried() {
            let r = post(&serve(vec![529, 200]));
            assert_eq!(r.status, 200);
            assert_eq!(r.retries, 1);
        }

        /// Backpressure that never lets up is reported, not retried forever - and the body
        /// survives, because responsibility 5 needs it.
        #[test]
        fn backpressure_that_never_lets_up_is_returned_after_three_attempts() {
            let r = post(&serve(vec![503, 503, 503]));
            assert_eq!(r.status, 503);
            assert_eq!(r.retries, 2, "three attempts is two retries");
            assert!(r.body.contains("503"), "the body reaches the mapper");
        }

        /// A fault is not backpressure: one attempt, no wait.
        #[test]
        fn a_500_is_returned_on_the_first_attempt() {
            let r = post(&serve(vec![500, 200]));
            assert_eq!(r.status, 500);
            assert_eq!(r.retries, 0);
        }
    }
}
