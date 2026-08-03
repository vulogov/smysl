//! The event loop and the terminal it borrows.
//!
//! Synchronous throughout (§21.5). The loop blocks on `crossterm::event::read`, which is
//! the whole of its concurrency: no runtime, no tasks, nothing to poll.
//!
//! The terminal is *borrowed*, and the borrowing is the delicate part. Raw mode and the
//! alternate screen are global state on a shared device, so a program that exits without
//! restoring them leaves the user with a shell that does not echo. `Guard` restores on
//! drop, which covers the return path, the `?` path, and a panic - the three ways out.

use std::io::{self, Stdout, Write};

use crossterm::event::{self, Event};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{Action, App};
use crate::draw::draw;

/// Restores the terminal however the program leaves.
///
/// Not a convenience: a panic inside the draw path is exactly when the terminal is in raw
/// mode with the alternate screen up, and exactly when nobody is in a position to type the
/// command that would fix it.
struct Guard;

impl Guard {
    fn enter() -> io::Result<Guard> {
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        Ok(Guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Errors are deliberately swallowed: this runs while unwinding, and a failure to
        // restore is not something a second failure helps with.
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
        let _ = io::stdout().flush();
    }
}

/// Run the UI until the user quits.
///
/// Returns once [`App::should_quit`] is set. Every path out restores the terminal.
pub fn run(mut app: App) -> io::Result<()> {
    let _guard = Guard::enter()?;
    let backend: CrosstermBackend<Stdout> = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    term.clear()?;

    loop {
        term.draw(|f| draw(f, &app))?;
        if app.should_quit() {
            return Ok(());
        }

        match event::read()? {
            // `Press` only. A terminal that reports releases and repeats would otherwise
            // act on every key three times, which on the budget dial is very visible.
            Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                app.update(Action::from_key(key));
            }
            // A resize needs a redraw and nothing else; the layout is computed per frame.
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}
