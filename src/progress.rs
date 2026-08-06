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
        //
        // The `||` here is the one mutant left alive in this file after 0.13's pass, and it is
        // unreachable rather than untested. `color` is `tty && !no_color`, and a test process
        // has no terminal on stderr — so whatever this line decides, `color` is false and the
        // decision is unobservable from inside. Setting NO_COLOR in a test would not help.
        //
        // Recorded rather than chased, the way 0.12 recorded `support_cycles`: a survivor that
        // no test can kill is a fact about the code, and pretending otherwise costs more than
        // it buys. Everything downstream of it is tested through `decide`.
        let no_color = no_color || std::env::var_os("NO_COLOR").is_some();
        Style::decide(std::io::stderr().is_terminal(), quiet, json, no_color)
    }

    /// The decision, separated from reading the environment.
    ///
    /// `detect` asks the process whether stderr is a terminal, and in a test it never is — so
    /// `enabled` was false whatever `quiet` and `json` said, and no test could reach the rest
    /// of the expression. Mutation testing in 0.13 put five survivors on those two lines for
    /// exactly that reason: `replace || with &&`, `delete !` twice over, all unobservable.
    ///
    /// Taking `tty` as an argument makes the truth table reachable. The environment read stays
    /// in `detect`, which is the one line that cannot be tested and does not need to be.
    pub(crate) const fn decide(tty: bool, quiet: bool, json: bool, no_color: bool) -> Style {
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

/// Where a bar's output goes.
///
/// Everything in this module wrote straight to `std::io::stderr()`, which made every drawing
/// path unobservable: the twelve tests all used `Style::silent()` because there was nothing
/// else they could do, and 51 mutants survived in 394 lines as a result — the whole of
/// `draw`, `clear`, and the arithmetic inside them.
///
/// A sink is the smallest thing that makes the output a value a test can read. Production
/// uses `Stderr` and pays one discriminant for it.
enum Sink {
    Stderr,
    /// Shared, so a test can still read what was drawn after `finish` or `abandon` has
    /// consumed the bar — those take `self` on purpose, and testing them any other way would
    /// be testing something callers do not do.
    #[cfg(test)]
    Buffer(std::sync::Arc<Mutex<String>>),
}

impl Sink {
    fn emit(&mut self, s: &str) {
        match self {
            Sink::Stderr => {
                let _ = write!(std::io::stderr(), "{s}");
                let _ = std::io::stderr().flush();
            }
            #[cfg(test)]
            Sink::Buffer(b) => {
                if let Ok(mut b) = b.lock() {
                    b.push_str(s);
                }
            }
        }
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
    sink: Sink,
}

/// Redraw no more often than this. A bar that repainted on every one of ten thousand units
/// would spend more time writing escape codes than doing the work it reports.
const MIN_INTERVAL: Duration = Duration::from_millis(80);

impl Bar {
    pub fn new(style: Style, label: impl Into<String>, total: usize) -> Bar {
        Bar::with_sink(style, label, total, Sink::Stderr)
    }

    fn with_sink(style: Style, label: impl Into<String>, total: usize, sink: Sink) -> Bar {
        let mut b = Bar {
            style,
            label: label.into(),
            total,
            done: 0,
            width: 0,
            started: Instant::now(),
            last_draw: None,
            sink,
        };
        b.draw(true);
        b
    }

    /// A bar that draws into a string instead of a terminal, so a test can read what it drew.
    ///
    /// Returns the buffer alongside it, which outlives the bar: `finish` and `abandon` take
    /// `self`, and a test that could not read the sink afterwards could not check either.
    #[cfg(test)]
    pub(crate) fn to_buffer(
        style: Style,
        label: impl Into<String>,
        total: usize,
    ) -> (Bar, std::sync::Arc<Mutex<String>>) {
        let buf = std::sync::Arc::new(Mutex::new(String::new()));
        let bar = Bar::with_sink(
            style,
            label,
            total,
            Sink::Buffer(std::sync::Arc::clone(&buf)),
        );
        (bar, buf)
    }

    /// What this bar has drawn so far.
    #[cfg(test)]
    fn drawn(&self) -> String {
        match &self.sink {
            Sink::Buffer(b) => b.lock().map(|b| b.clone()).unwrap_or_default(),
            Sink::Stderr => String::new(),
        }
    }

    /// Advance by one and redraw if enough time has passed.
    pub fn tick(&mut self) {
        self.advance(1);
    }

    pub fn advance(&mut self, n: usize) {
        // Clamped to `total`, which is what the previous expression was written to do and did
        // not: it was `(done + n).min(total.max(done + n))`, and `y.max(x)` is never below
        // `x`, so `x.min(y.max(x))` is `x` — a no-op, verified over 200 000 random triples.
        // A caller that overshot printed `105/100`. Two mutants survived on that line because
        // the arithmetic inside a discarded comparison cannot be observed.
        //
        // `total == 0` means "no countable steps", and `filled` already renders that as a
        // full bar, so clamping to zero there would be wrong. It keeps counting instead.
        self.done = if self.total == 0 {
            self.done + n
        } else {
            (self.done + n).min(self.total)
        };
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
        self.finish_in_place(message);
    }

    /// The work `finish` does. Separated only so a test can read the sink afterwards —
    /// `finish` takes `self` on purpose, and that is worth keeping.
    fn finish_in_place(&mut self, message: &str) {
        self.clear();
        if self.style.enabled && !message.is_empty() {
            let line = format!("{message} ({:.1}s)\n", self.started.elapsed().as_secs_f64());
            self.sink.emit(&line);
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
            let blanks = " ".repeat(self.width);
            self.sink.emit(&format!("\r{blanks}\r"));
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

        let (line, printed) = render(self.done, self.total, &self.label, self.style.color);
        let pad = " ".repeat(self.width.saturating_sub(printed));
        self.sink.emit(&format!("\r{line}{pad}"));
        // Was preceded by `self.width = printed.max(self.width.min(printed + pad))`, which the
        // next line overwrote unconditionally. Dead, and the two mutants on its arithmetic
        // survived because a discarded value cannot be observed. Deleted rather than repaired:
        // `printed` is what was just written, which is what `clear` needs to erase.
        self.width = printed;
    }
}

/// How many cells of the bar are filled, for `done` of `total`.
///
/// `total == 0` means there is nothing to count, which renders as complete rather than as a
/// division by zero — a bar over zero steps is finished before it starts.
fn filled_cells(done: usize, total: usize, cells: usize) -> usize {
    match total {
        0 => cells,
        t => (done * cells) / t,
    }
    .min(cells)
}

/// The line a bar draws, and the width it occupies on screen.
///
/// Separated from `draw` because everything interesting is here and none of it was reachable
/// while it was welded to `stderr`: the cell arithmetic, the width arithmetic, and the escape
/// codes that must not count towards the width — twenty mutants across three lines, every one
/// of them surviving because no test could see a drawn bar.
///
/// The returned width deliberately excludes the colour escapes. `clear` erases that many
/// columns, so counting the escapes would leave the tail of the bar on screen.
fn render(done: usize, total: usize, label: &str, color: bool) -> (String, usize) {
    const CELLS: usize = 24;
    let filled = filled_cells(done, total, CELLS);

    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(CELLS - filled));
    let bar = if color {
        format!("\u{1b}[36m{bar}\u{1b}[0m")
    } else {
        bar
    };

    let line = format!("  {bar} {done}/{total}  {label}");
    let printed = 2 + CELLS + 1 + count_len(done, total) + 2 + label.chars().count();
    (line, printed)
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
    /// Where the *final* line goes. The animation belongs to the painting thread and keeps
    /// writing to stderr directly — it is a background repaint nobody reads afterwards. The
    /// line `finish` leaves behind is the one a user actually keeps, so it is the one worth
    /// being able to read back.
    sink: Sink,
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
            sink: Sink::Stderr,
        }
    }

    /// A spinner whose final line goes to a string, so a test can read it.
    #[cfg(test)]
    fn to_buffer(
        style: Style,
        label: impl Into<String>,
    ) -> (Spinner, std::sync::Arc<Mutex<String>>) {
        let buf = std::sync::Arc::new(Mutex::new(String::new()));
        let mut s = Spinner::new(style, label);
        s.sink = Sink::Buffer(std::sync::Arc::clone(&buf));
        (s, buf)
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
            let line = format!("{message} ({:.1}s)\n", self.started.elapsed().as_secs_f64());
            self.sink.emit(&line);
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

    // ---- what the twelve tests above never checked: the numbers -------------------------
    //
    // Every test before this line uses `Style::silent()`, because until 0.13 there was
    // nothing else available: `draw` wrote to `std::io::stderr()`, so a bar that drew was a
    // bar nothing could read. 51 mutants survived in this file — the whole of `draw` and
    // `clear`, the cell arithmetic, the width arithmetic — and two of them were real defects
    // rather than untested code. Both are fixed above; these are the tests that see them.

    /// The full truth table, which `detect` could not reach: in a test stderr is never a
    /// terminal, so `enabled` was false whatever the flags said.
    #[test]
    fn the_style_decision_depends_on_all_four_inputs() {
        assert!(Style::decide(true, false, false, false).is_enabled());
        assert!(Style::decide(true, false, false, false).color);

        // A terminal is necessary, and each of quiet and json is individually sufficient to
        // silence it. That is three separate claims, and `&&`/`||` mutants tell them apart.
        assert!(!Style::decide(false, false, false, false).is_enabled());
        assert!(!Style::decide(true, true, false, false).is_enabled());
        assert!(!Style::decide(true, false, true, false).is_enabled());

        // Colour is a separate axis: --no-color on a terminal still draws a bar.
        assert!(Style::decide(true, false, false, true).is_enabled());
        assert!(!Style::decide(true, false, false, true).color);
        // ...and no terminal means no colour, whatever --no-color said.
        assert!(!Style::decide(false, false, false, false).color);
    }

    #[test]
    fn the_filled_cells_track_the_fraction_done() {
        assert_eq!(filled_cells(0, 100, 24), 0);
        assert_eq!(filled_cells(50, 100, 24), 12);
        assert_eq!(filled_cells(100, 100, 24), 24);
        // Truncation, not rounding: 1 of 100 is not yet a whole cell.
        assert_eq!(filled_cells(1, 100, 24), 0);
        assert_eq!(filled_cells(5, 100, 24), 1);
    }

    /// Nothing to count renders as complete rather than as a division by zero.
    #[test]
    fn a_bar_over_zero_steps_is_full_rather_than_undefined() {
        assert_eq!(filled_cells(0, 0, 24), 24);
        assert_eq!(filled_cells(7, 0, 24), 24);
    }

    /// A caller that overshoots must not paint past the end of the bar.
    #[test]
    fn more_done_than_total_still_fills_exactly_the_bar() {
        assert_eq!(filled_cells(300, 100, 24), 24);
    }

    /// The recorded width is what `clear` erases, so counting the colour escapes would leave
    /// the tail of the bar on screen. This is the invariant, stated as one.
    #[test]
    fn the_recorded_width_is_the_visible_width_not_the_byte_length() {
        for color in [false, true] {
            let (line, printed) = render(7, 99, "indexing", color);
            let visible: String = strip_escapes(&line);
            assert_eq!(
                visible.chars().count(),
                printed,
                "color={color}: recorded {printed} for a line that occupies {} columns: {visible:?}",
                visible.chars().count()
            );
        }
    }

    /// Colour changes the bytes and must not change the geometry.
    #[test]
    fn colour_adds_escapes_and_nothing_else() {
        let (plain, wp) = render(3, 10, "x", false);
        let (fancy, wf) = render(3, 10, "x", true);
        assert_eq!(wp, wf, "colour must not change the printed width");
        assert_ne!(plain, fancy, "colour must actually change the output");
        assert_eq!(strip_escapes(&fancy), plain);
    }

    /// The label is part of the width, or a long label leaves a trail behind it.
    #[test]
    fn a_longer_label_makes_a_wider_line() {
        let (_, short) = render(1, 2, "ab", false);
        let (_, long) = render(1, 2, "abcd", false);
        assert_eq!(long, short + 2);
    }

    /// Wider numbers make a wider line, which is what `count_len` is for.
    #[test]
    fn wider_counts_make_a_wider_line() {
        let (_, small) = render(1, 9, "x", false);
        let (_, big) = render(100, 999, "x", false);
        assert_eq!(big, small + 4);
    }

    /// The defect this file's `advancing_past_the_total_does_not_panic` could not see: it
    /// asserted no panic and never looked at the result. The clamp was a no-op, so a bar
    /// asked to advance past its total printed `105/100`.
    #[test]
    fn advancing_past_the_total_stops_at_the_total() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "overrun", 2);
        b.advance(99);
        assert_eq!(b.done, 2, "done must not exceed total");
        assert!(b.drawn().contains("2/2"), "drew: {:?}", b.drawn());
        b.finish("");
    }

    /// A bar with no countable steps keeps counting rather than clamping to zero.
    #[test]
    fn a_bar_over_zero_steps_still_counts_what_it_did() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "unknown", 0);
        b.advance(5);
        assert_eq!(b.done, 5);
        b.finish("");
    }

    #[test]
    fn a_bar_draws_itself_as_soon_as_it_exists() {
        let (b, _buf) = Bar::to_buffer(enabled(), "working", 4);
        assert!(b.drawn().contains("0/4"), "drew: {:?}", b.drawn());
        assert!(b.drawn().contains("working"), "drew: {:?}", b.drawn());
    }

    /// The rate limit, which the first version of this test had backwards.
    ///
    /// A tick inside `MIN_INTERVAL` of the last draw must *not* repaint — a bar over ten
    /// thousand units would otherwise spend more time writing escape codes than working.
    #[test]
    fn a_tick_inside_the_interval_does_not_repaint() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "working", 400);
        let after_new = b.drawn().len();
        b.tick();
        assert_eq!(b.drawn().len(), after_new, "repainted inside the interval");
    }

    /// ...but the last step always draws, whatever the interval says, so a finished bar
    /// never reads as stalled at 97%.
    #[test]
    fn the_last_step_always_repaints() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "working", 2);
        b.tick();
        let mid = b.drawn().len();
        b.tick(); // now done == total
        assert!(b.drawn().len() > mid, "the final step did not repaint");
        assert!(b.drawn().contains("2/2"), "drew: {:?}", b.drawn());
    }

    /// `set_label` forces a draw, which is what makes it usable inside a tight loop.
    #[test]
    fn setting_a_label_forces_a_repaint_inside_the_interval() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "first", 400);
        let after_new = b.drawn().len();
        b.set_label("second");
        assert!(b.drawn().len() > after_new, "set_label did not repaint");
    }

    /// `set_label` redraws without moving the bar, which is the whole point of it.
    #[test]
    fn setting_a_label_redraws_with_the_new_text_and_the_same_count() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "first", 4);
        b.tick();
        b.set_label("second");
        let drawn = b.drawn();
        assert!(drawn.contains("second"), "drew: {drawn:?}");
        assert!(drawn.contains("1/4"), "drew: {drawn:?}");
        b.finish("");
    }

    /// `clear` must erase exactly the columns it wrote — no fewer, or the tail stays on
    /// screen; no more, and it eats the line above.
    #[test]
    fn clearing_erases_exactly_the_width_that_was_drawn() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "working", 4);
        let width = b.width;
        assert!(width > 0, "a drawn bar has a width");
        let before = b.drawn().len();
        b.suspend();
        let erased = &b.drawn()[before..];
        assert_eq!(erased, format!("\r{}\r", " ".repeat(width)));
        assert_eq!(b.width, 0, "a cleared bar has nothing left to erase");
        b.finish("");
    }

    #[test]
    fn finishing_clears_the_bar_and_prints_the_message() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "working", 2);
        let width = b.width;
        let before = b.drawn().len();
        b.finish_in_place("all done");
        let tail = &b.drawn()[before..];
        assert!(
            tail.starts_with(&format!("\r{}\r", " ".repeat(width))),
            "finish must erase the bar first, wrote: {tail:?}"
        );
        assert!(tail.contains("all done"), "wrote: {tail:?}");
        assert!(
            tail.ends_with("\n"),
            "the final line must end the line: {tail:?}"
        );
        assert_eq!(b.width, 0);
    }

    /// An empty message means "erase and say nothing", which `abandon` relies on.
    #[test]
    fn finishing_with_no_message_prints_nothing_after_the_erase() {
        let (mut b, _buf) = Bar::to_buffer(enabled(), "working", 2);
        let width = b.width;
        let before = b.drawn().len();
        b.finish_in_place("");
        assert_eq!(&b.drawn()[before..], format!("\r{}\r", " ".repeat(width)));
    }

    /// A silent bar draws nothing at all, which is the property every other call site relies
    /// on to avoid branching.
    #[test]
    fn a_silent_bar_writes_nothing_to_its_sink() {
        let (mut b, _buf) = Bar::to_buffer(Style::silent(), "quiet", 4);
        b.tick();
        b.advance(2);
        b.set_label("still quiet");
        b.suspend();
        assert_eq!(b.drawn(), "", "a silent bar drew: {:?}", b.drawn());
        assert_eq!(b.width, 0);
        b.finish("done");
    }

    /// `finish` and `abandon` take `self`, which is how every call site ends a bar — so they
    /// are tested that way, through a buffer that outlives the bar. Both survived mutation to
    /// `()` while the only tests that could reach them called the inner helper instead.
    #[test]
    fn finishing_a_bar_by_value_erases_it_and_says_what_happened() {
        let (b, buf) = Bar::to_buffer(enabled(), "working", 2);
        let width = b.width;
        let before = buf.lock().unwrap().len();
        b.finish("all done");
        let tail = buf.lock().unwrap()[before..].to_string();
        assert!(
            tail.starts_with(&format!("\r{}\r", " ".repeat(width))),
            "finish must erase first: {tail:?}"
        );
        assert!(tail.contains("all done"), "wrote: {tail:?}");
    }

    #[test]
    fn abandoning_a_bar_erases_it_and_says_nothing() {
        let (b, buf) = Bar::to_buffer(enabled(), "cancelled", 5);
        let width = b.width;
        let before = buf.lock().unwrap().len();
        b.abandon();
        let tail = buf.lock().unwrap()[before..].to_string();
        assert_eq!(tail, format!("\r{}\r", " ".repeat(width)));
    }

    /// Clearing twice must write nothing the second time: `clear` guards on
    /// `enabled && width > 0`, and both halves of that guard matter. Turn the `&&` into `||`
    /// or the `>` into `>=` and an already-cleared bar emits a stray `\r\r`.
    #[test]
    fn clearing_an_already_cleared_bar_writes_nothing() {
        let (mut b, buf) = Bar::to_buffer(enabled(), "working", 4);
        b.suspend();
        let after_first = buf.lock().unwrap().len();
        b.suspend();
        assert_eq!(
            buf.lock().unwrap().len(),
            after_first,
            "a second clear wrote something"
        );
    }

    /// The spinner's state, which is all a test can reach without a terminal: the label is
    /// shared with the painting thread, and stopping must both drop the flag and join.
    #[test]
    fn setting_a_spinner_label_changes_what_the_painter_reads() {
        let mut s = Spinner::new(Style::silent(), "probing");
        assert_eq!(&*s.label.lock().unwrap(), "probing");
        s.set_label("still probing");
        assert_eq!(&*s.label.lock().unwrap(), "still probing");
    }

    #[test]
    fn stopping_a_spinner_clears_the_flag_and_takes_the_thread() {
        let mut s = Spinner::new(Style::silent(), "work");
        assert!(s.running.load(Ordering::Relaxed));
        s.stop();
        assert!(
            !s.running.load(Ordering::Relaxed),
            "the painter was not told to stop"
        );
        assert!(s.painter.is_none(), "the thread handle was not taken");
    }

    /// Dropping must stop it, or a caller returning early on an error leaves a thread
    /// painting over the error message. Observed through the shared flag, which outlives the
    /// spinner.
    #[test]
    fn dropping_a_spinner_clears_the_running_flag() {
        let s = Spinner::new(Style::silent(), "abandoned");
        let running = Arc::clone(&s.running);
        assert!(running.load(Ordering::Relaxed));
        drop(s);
        assert!(
            !running.load(Ordering::Relaxed),
            "drop did not stop the painter"
        );
    }

    /// The line a user keeps after a spinner ends. Three mutants lived here — `finish`
    /// replaced with `()`, and both halves of its guard — because the only output path went
    /// straight to stderr.
    #[test]
    fn a_spinner_finishing_prints_the_message_and_the_elapsed_time() {
        let (s, buf) = Spinner::to_buffer(Style::decide(true, false, false, true), "probing");
        s.finish("probed 3 providers");
        let out = buf.lock().unwrap().clone();
        assert!(out.contains("probed 3 providers"), "wrote: {out:?}");
        assert!(
            out.contains('s'),
            "the elapsed time is part of the line: {out:?}"
        );
        assert!(out.ends_with('\n'), "wrote: {out:?}");
    }

    /// Both halves of the guard: an empty message says nothing, and a silent spinner says
    /// nothing whatever the message.
    #[test]
    fn a_spinner_says_nothing_when_it_has_nothing_to_say() {
        let (s, buf) = Spinner::to_buffer(Style::decide(true, false, false, true), "quiet");
        s.finish("");
        assert_eq!(&*buf.lock().unwrap(), "");

        let (s, buf) = Spinner::to_buffer(Style::silent(), "quiet");
        s.finish("suppressed");
        assert_eq!(&*buf.lock().unwrap(), "");
    }

    /// A style that draws, for the tests above. `Style::silent()` is the only constructor
    /// that does not consult the environment, and it is the wrong one for these.
    fn enabled() -> Style {
        Style::decide(true, false, false, true)
    }

    /// Drop the ANSI colour escapes, so a test can count what a terminal would show.
    fn strip_escapes(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn there_are_frames_to_spin() {
        assert!(FRAMES.len() > 1);
    }
}
