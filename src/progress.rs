//! Progress reporting for the CLI.
//!
//! Three rules, and they are all about not lying to the terminal:
//!
//! 1. **Progress goes to stderr.** stdout carries the artifact - a store, a pack, a
//!    rendered document - and a pipeline that received a progress bar in the middle of a
//!    CBOR sequence would be reading corruption. Rule P says stdout defaults to CBOR on a
//!    non-TTY; progress must not appear there under any circumstances.
//!
//! 2. **Nothing is drawn unless stderr is a terminal.** A log file full of `\r` and spinner
//!    frames is worse than no progress at all, and CI logs are the usual victim.
//!
//! 3. **`--quiet` silences it, `--json` silences it.** A caller asking for machine-readable
//!    output is not asking to watch.
//!
//! No dependency: a bar is a carriage return, some blocks, and a count.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Whether progress may be drawn at all.
///
/// Decided once, from the environment and the flags, and carried into every bar - so a
/// command cannot accidentally draw in one place and not another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    enabled: bool,
    /// Colour is separate from drawing: `--no-color` on a terminal still gets a bar.
    color: bool,
}

impl Style {
    pub fn detect(quiet: bool, json: bool, no_color: bool) -> Style {
        // NO_COLOR is a de-facto standard and costs one line to honour.
        let no_color = no_color || std::env::var_os("NO_COLOR").is_some();
        let tty = std::io::stderr().is_terminal();
        Style {
            enabled: tty && !quiet && !json,
            color: tty && !no_color,
        }
    }

    /// Progress that is never drawn, for tests and for library callers.
    pub const fn silent() -> Style {
        Style {
            enabled: false,
            color: false,
        }
    }

    /// Whether anything will be drawn.
    ///
    /// A caller with slow work and no bar may want to say so once instead. Only the
    /// provider commands need that today, hence the conditional allow rather than a
    /// blanket one - the method is real API, not scaffolding.
    #[cfg_attr(not(feature = "providers"), allow(dead_code))]
    pub const fn is_enabled(self) -> bool {
        self.enabled
    }
}

/// A bar over a known number of steps.
pub struct Bar {
    style: Style,
    label: String,
    total: usize,
    done: usize,
    /// The last line drawn, so `clear` knows how much to erase.
    width: usize,
    started: Instant,
    last_draw: Option<Instant>,
}

/// Redraw no more often than this. A bar that repainted on every one of ten thousand units
/// would spend more time writing escape codes than doing the work it reports.
const MIN_INTERVAL: Duration = Duration::from_millis(80);

impl Bar {
    pub fn new(style: Style, label: impl Into<String>, total: usize) -> Bar {
        let mut b = Bar {
            style,
            label: label.into(),
            total,
            done: 0,
            width: 0,
            started: Instant::now(),
            last_draw: None,
        };
        b.draw(true);
        b
    }

    /// Advance by one and redraw if enough time has passed.
    pub fn tick(&mut self) {
        self.advance(1);
    }

    pub fn advance(&mut self, n: usize) {
        self.done = (self.done + n).min(self.total.max(self.done + n));
        self.draw(false);
    }

    /// Change what the bar says it is doing, without moving it.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
        self.draw(true);
    }

    /// Erase the bar and print a final line in its place.
    ///
    /// Taking `self` means a bar cannot be left half-drawn on the terminal: the only way to
    /// stop reporting is to say what happened.
    pub fn finish(mut self, message: &str) {
        self.clear();
        if self.style.enabled && !message.is_empty() {
            let _ = writeln!(
                std::io::stderr(),
                "{message} ({:.1}s)",
                self.started.elapsed().as_secs_f64()
            );
        }
    }

    /// Erase the bar and say nothing.
    pub fn abandon(mut self) {
        self.clear();
    }

    /// Erase the bar so the caller can print, without ending it.
    ///
    /// A command that emits diagnostics as it goes would otherwise interleave them with
    /// the bar and leave fragments of both on the terminal. The next `tick` or `set_label`
    /// redraws it.
    pub fn suspend(&mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        if self.style.enabled && self.width > 0 {
            let _ = write!(std::io::stderr(), "\r{}\r", " ".repeat(self.width));
            let _ = std::io::stderr().flush();
            self.width = 0;
        }
    }

    fn draw(&mut self, force: bool) {
        if !self.style.enabled {
            return;
        }
        let now = Instant::now();
        let due = match self.last_draw {
            Some(t) => now.duration_since(t) >= MIN_INTERVAL,
            None => true,
        };
        // The last step always draws, so a finished bar never reads as stalled at 97%.
        if !force && !due && self.done < self.total {
            return;
        }
        self.last_draw = Some(now);

        const CELLS: usize = 24;
        let filled = match self.total {
            0 => CELLS,
            t => (self.done * CELLS) / t.max(1),
        }
        .min(CELLS);

        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(CELLS - filled));
        let bar = if self.style.color {
            format!("\u{1b}[36m{bar}\u{1b}[0m")
        } else {
            bar
        };

        let line = format!(
            "  {bar} {done}/{total}  {label}",
            done = self.done,
            total = self.total,
            label = self.label
        );
        // The printed width excludes the escape codes, or `clear` would erase too little.
        let printed =
            2 + CELLS + 1 + count_len(self.done, self.total) + 2 + self.label.chars().count();

        let pad = self.width.saturating_sub(printed);
        let _ = write!(std::io::stderr(), "\r{line}{}", " ".repeat(pad));
        let _ = std::io::stderr().flush();
        self.width = printed.max(self.width.min(printed + pad));
        self.width = printed;
    }
}

fn count_len(done: usize, total: usize) -> usize {
    let d = |n: usize| n.to_string().chars().count();
    d(done) + 1 + d(total)
}

/// A spinner for work with no countable steps - a branch-and-bound search, a model call.
///
/// It reports elapsed time rather than a percentage, because a bar that invented a
/// denominator would be a bar that lied about how far along it was.
///
/// The work it reports is *blocking* - that is why it has no steps to count - so the
/// animation runs on its own thread. A spinner that only redrew when the caller got round
/// to calling `tick` would sit frozen for exactly the interval it exists to fill.
pub struct Spinner {
    style: Style,
    label: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    painter: Option<std::thread::JoinHandle<()>>,
    started: Instant,
}

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl Spinner {
    pub fn new(style: Style, label: impl Into<String>) -> Spinner {
        let label = Arc::new(Mutex::new(label.into()));
        let running = Arc::new(AtomicBool::new(true));
        let started = Instant::now();

        let painter = style.enabled.then(|| {
            let label = Arc::clone(&label);
            let running = Arc::clone(&running);
            let color = style.color;
            std::thread::spawn(move || {
                let mut frame = 0usize;
                let mut width = 0usize;
                while running.load(Ordering::Relaxed) {
                    let text = label.lock().map(|l| l.clone()).unwrap_or_default();
                    let secs = format!("{:.0}s", started.elapsed().as_secs_f64());
                    let f = FRAMES[frame % FRAMES.len()];
                    let painted = if color {
                        format!("\u{1b}[36m{f}\u{1b}[0m")
                    } else {
                        f.to_string()
                    };
                    let printed = 2 + 1 + 1 + text.chars().count() + 3 + secs.chars().count();
                    let pad = width.saturating_sub(printed);
                    let _ = write!(
                        std::io::stderr(),
                        "\r  {painted} {text} … {secs}{}",
                        " ".repeat(pad)
                    );
                    let _ = std::io::stderr().flush();
                    width = printed;
                    frame += 1;
                    std::thread::sleep(FRAME_INTERVAL);
                }
                // The painter owns the line, so the painter erases it - no other thread
                // knows how wide it grew.
                if width > 0 {
                    let _ = write!(std::io::stderr(), "\r{}\r", " ".repeat(width));
                    let _ = std::io::stderr().flush();
                }
            })
        });

        Spinner {
            style,
            label,
            running,
            painter,
            started,
        }
    }

    pub fn set_label(&mut self, label: impl Into<String>) {
        if let Ok(mut l) = self.label.lock() {
            *l = label.into();
        }
    }

    /// Stop the animation and print a final line in its place.
    pub fn finish(mut self, message: &str) {
        self.stop();
        if self.style.enabled && !message.is_empty() {
            let _ = writeln!(
                std::io::stderr(),
                "{message} ({:.1}s)",
                self.started.elapsed().as_secs_f64()
            );
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.painter.take() {
            let _ = h.join();
        }
    }
}

/// Dropping a spinner must stop it: a caller that returns early on an error would otherwise
/// leave a thread painting over the error message.
impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

const FRAME_INTERVAL: Duration = Duration::from_millis(90);

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is drawn when stderr is not a terminal - which, in a test, it never is.
    #[test]
    fn progress_is_disabled_when_stderr_is_not_a_terminal() {
        assert!(!Style::detect(false, false, false).is_enabled());
    }

    #[test]
    fn quiet_and_json_disable_progress() {
        assert!(!Style::detect(true, false, false).is_enabled());
        assert!(!Style::detect(false, true, false).is_enabled());
    }

    #[test]
    fn a_silent_style_draws_nothing() {
        assert!(!Style::silent().is_enabled());
    }

    /// A silent bar must still be usable, or every call site would need a branch.
    #[test]
    fn a_silent_bar_accepts_every_operation() {
        let mut b = Bar::new(Style::silent(), "working", 3);
        b.tick();
        b.advance(2);
        b.set_label("still working");
        b.finish("done");

        let mut s = Spinner::new(Style::silent(), "probing");
        s.set_label("still probing");
        s.finish("done");
    }

    #[test]
    fn a_suspended_bar_can_be_resumed() {
        let mut b = Bar::new(Style::silent(), "working", 3);
        b.tick();
        b.suspend();
        b.tick();
        b.finish("done");
    }

    /// A caller that returns early on an error must not leave a thread painting over the
    /// error message.
    #[test]
    fn dropping_a_spinner_stops_it() {
        let s = Spinner::new(Style::silent(), "abandoned");
        drop(s);
    }

    #[test]
    fn a_silent_spinner_starts_no_thread() {
        let s = Spinner::new(Style::silent(), "quiet");
        assert!(s.painter.is_none());
        s.finish("");
    }

    #[test]
    fn a_bar_over_zero_steps_does_not_divide_by_zero() {
        let mut b = Bar::new(Style::silent(), "nothing to do", 0);
        b.tick();
        b.finish("");
    }

    #[test]
    fn advancing_past_the_total_does_not_panic() {
        let mut b = Bar::new(Style::silent(), "overrun", 2);
        b.advance(99);
        b.finish("");
    }

    #[test]
    fn an_abandoned_bar_prints_nothing() {
        Bar::new(Style::silent(), "cancelled", 5).abandon();
    }

    #[test]
    fn the_count_width_covers_both_numbers_and_the_slash() {
        assert_eq!(count_len(1, 9), 3);
        assert_eq!(count_len(10, 100), 6);
    }

    #[test]
    fn there_are_frames_to_spin() {
        assert!(FRAMES.len() > 1);
    }
}
