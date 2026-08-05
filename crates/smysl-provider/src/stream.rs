//! The streaming bridge (§21.5).
//!
//! Streaming crosses from the provider thread to a synchronous caller over an
//! `std::sync::mpsc` channel. The TUI drains it with `try_recv` inside its crossterm loop,
//! so no async ever appears in an event path and no second concurrency model is introduced.
//!
//! The channel is the contract. What produces the messages - a blocking read loop today,
//! a future tomorrow - is behind it.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use smysl_core::error::ProviderError;

use crate::Usage;

/// One message from a running completion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamMsg {
    Token(String),
    Done(Usage),
    Error(ProviderError),
}

/// The receiving half, with the draining a synchronous caller actually wants.
/// Reachable for `tests/ollama_live.rs`, which drives a real streaming response and cannot do
/// that from inside the crate. Hidden per §1.2 S2; a consumer streams through
/// `Provider::stream` and never holds this.
///
/// Hidden on the type rather than on the module, which is where 0.13 first put it. The module
/// also holds `StreamMsg`, which *is* contract — the facade exports it and `Provider::stream`
/// takes a channel of it. Hiding the module hid the enum too: `cargo public-api` still saw it
/// through the root `pub use` and reported no change, while `cargo-semver-checks` counted
/// `enum_now_doc_hidden`, which is removal from the API. Two gates disagreeing about one type
/// is the answer being wrong, and §1.2 S6's audit is what surfaced it.
#[doc(hidden)]
pub struct Stream {
    rx: Receiver<StreamMsg>,
    text: String,
    usage: Option<Usage>,
    error: Option<ProviderError>,
    finished: bool,
}

impl Stream {
    pub fn new(rx: Receiver<StreamMsg>) -> Stream {
        Stream {
            rx,
            text: String::new(),
            usage: None,
            error: None,
            finished: false,
        }
    }

    /// Take everything available without blocking, returning the new text.
    ///
    /// This is what an event loop calls once per frame. It never blocks, so a stalled
    /// provider slows the completion down and not the interface.
    pub fn drain(&mut self) -> String {
        let mut fresh = String::new();
        loop {
            match self.rx.try_recv() {
                Ok(StreamMsg::Token(t)) => {
                    fresh.push_str(&t);
                    self.text.push_str(&t);
                }
                Ok(StreamMsg::Done(u)) => {
                    self.usage = Some(u);
                    self.finished = true;
                }
                Ok(StreamMsg::Error(e)) => {
                    self.error = Some(e);
                    self.finished = true;
                }
                // A dropped sender without a `Done` is a provider that died mid-stream.
                // That is unreachable rather than complete: the caller has partial text
                // and must not treat it as an answer.
                Err(TryRecvError::Disconnected) => {
                    if !self.finished {
                        self.error = Some(ProviderError::Unreachable);
                        self.finished = true;
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        fresh
    }

    /// Block until the stream ends, returning everything it produced.
    pub fn collect(mut self) -> Result<(String, Usage), ProviderError> {
        for msg in self.rx.iter() {
            match msg {
                StreamMsg::Token(t) => self.text.push_str(&t),
                StreamMsg::Done(u) => {
                    self.usage = Some(u);
                    self.finished = true;
                }
                StreamMsg::Error(e) => return Err(e),
            }
        }
        match (self.finished, self.usage) {
            (true, Some(u)) => Ok((self.text, u)),
            _ => Err(self.error.unwrap_or(ProviderError::Unreachable)),
        }
    }

    /// Everything received so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn usage(&self) -> Option<Usage> {
        self.usage
    }

    pub fn error(&self) -> Option<&ProviderError> {
        self.error.as_ref()
    }
}

/// A sender that counts what it emitted, so a mapper does not have to.
pub(crate) struct Emitter {
    tx: Sender<StreamMsg>,
    output_chars: usize,
}

impl Emitter {
    pub fn new(tx: Sender<StreamMsg>) -> Emitter {
        Emitter {
            tx,
            output_chars: 0,
        }
    }

    /// Send a token. A closed receiver is not an error - it is a cancelled operation, and
    /// the mapper should stop rather than fail.
    pub fn token(&mut self, t: &str) -> bool {
        self.output_chars += t.len();
        self.tx.send(StreamMsg::Token(t.to_string())).is_ok()
    }

    pub fn done(self, usage: Usage) {
        let _ = self.tx.send(StreamMsg::Done(usage));
    }

    pub fn fail(self, e: ProviderError) {
        let _ = self.tx.send(StreamMsg::Error(e));
    }

    /// Bytes of text emitted, for a provider that reports no usage of its own.
    pub fn output_chars(&self) -> usize {
        self.output_chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pair() -> (Emitter, Stream) {
        let (tx, rx) = mpsc::channel();
        (Emitter::new(tx), Stream::new(rx))
    }

    #[test]
    fn draining_returns_only_what_is_new() {
        let (mut e, mut s) = pair();
        e.token("hel");
        assert_eq!(s.drain(), "hel");
        e.token("lo");
        assert_eq!(
            s.drain(),
            "lo",
            "the second drain does not repeat the first"
        );
        assert_eq!(s.text(), "hello");
    }

    /// This is what an event loop does once per frame; it must never block.
    #[test]
    fn draining_an_idle_stream_returns_nothing_and_does_not_block() {
        let (_e, mut s) = pair();
        assert_eq!(s.drain(), "");
        assert!(!s.is_finished());
    }

    #[test]
    fn a_finished_stream_reports_its_usage() {
        let (mut e, mut s) = pair();
        e.token("x");
        e.done(Usage {
            output_tokens: 1,
            ..Usage::default()
        });
        s.drain();
        assert!(s.is_finished());
        assert_eq!(s.usage().map(|u| u.output_tokens), Some(1));
    }

    /// A provider that dies mid-stream leaves partial text, which must not be mistaken for
    /// an answer.
    #[test]
    fn a_dropped_sender_without_done_is_unreachable_not_complete() {
        let (e, mut s) = pair();
        drop(e);
        s.drain();
        assert!(s.is_finished());
        assert_eq!(s.error(), Some(&ProviderError::Unreachable));
    }

    #[test]
    fn collect_blocks_until_the_stream_ends() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut e = Emitter::new(tx);
            e.token("a");
            e.token("b");
            e.done(Usage {
                output_tokens: 2,
                ..Usage::default()
            });
        });
        let (text, usage) = Stream::new(rx).collect().unwrap();
        assert_eq!(text, "ab");
        assert_eq!(usage.output_tokens, 2);
    }

    #[test]
    fn collect_surfaces_an_error_rather_than_the_partial_text() {
        let (tx, rx) = mpsc::channel();
        let mut e = Emitter::new(tx);
        e.token("partial");
        e.fail(ProviderError::Unauthorized);
        assert_eq!(Stream::new(rx).collect(), Err(ProviderError::Unauthorized));
    }

    #[test]
    fn collect_without_a_done_message_is_an_error() {
        let (tx, rx) = mpsc::channel();
        let mut e = Emitter::new(tx);
        e.token("partial");
        drop(e);
        assert!(Stream::new(rx).collect().is_err());
    }

    /// A closed receiver is a cancelled operation, not a failure: the mapper should stop
    /// rather than report an error nobody is listening for.
    #[test]
    fn a_closed_receiver_tells_the_emitter_to_stop() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let mut e = Emitter::new(tx);
        assert!(!e.token("x"), "the emitter learns to stop");
    }

    #[test]
    fn the_emitter_counts_what_it_sent() {
        let (mut e, _s) = pair();
        e.token("hello");
        e.token(" world");
        assert_eq!(e.output_chars(), 11);
    }

    #[test]
    fn an_error_after_tokens_still_reaches_a_draining_caller() {
        let (mut e, mut s) = pair();
        e.token("x");
        e.fail(ProviderError::RateLimited { retry_after: None });
        s.drain();
        assert!(matches!(s.error(), Some(ProviderError::RateLimited { .. })));
        assert_eq!(s.text(), "x");
    }
}
