//! `smysl-tui` - the seven-pane terminal UI (§24).
//!
//! Synchronous crossterm loop. It never links a runtime: streaming model output arrives
//! from `smysl-provider` over an `std::sync::mpsc` channel drained with `try_recv`, so no
//! async appears in the event path (§21.5).
//!
//! The pack simulator is the pane that earns the TUI: budget behaviour is the least
//! intuitive part of the format, and a live dial showing the responsible constraint is
//! far more legible than `--explain` output.
//!
//! Filled by SM-P15.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

/// The panes of §24.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Pane {
    Graph,
    Detail,
    Thread,
    Contentions,
    Lineage,
    PackSimulator,
    Staging,
}

impl Pane {
    pub const ALL: &'static [Pane] = &[
        Pane::Graph,
        Pane::Detail,
        Pane::Thread,
        Pane::Contentions,
        Pane::Lineage,
        Pane::PackSimulator,
        Pane::Staging,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Pane::Graph => "Graph",
            Pane::Detail => "Detail",
            Pane::Thread => "Thread",
            Pane::Contentions => "Contentions",
            Pane::Lineage => "Lineage",
            Pane::PackSimulator => "Pack simulator",
            Pane::Staging => "Staging",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_panes_with_titles() {
        assert_eq!(Pane::ALL.len(), 7);
        for &p in Pane::ALL {
            assert!(!p.title().is_empty());
        }
    }
}

pub mod app;
pub use app::{Action, App, Sim};

pub mod draw;
pub use draw::render_to_string;

pub mod run;
pub use run::run;
