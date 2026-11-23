//! smysl's language server (RFC SMYSL-6): what `smysl check` and `smysl q` say about a document,
//! shown in the editor while it is written.
//!
//! - [`analysis`] is pure and synchronous: an [`analysis::AnalysisHost`] holds the open
//!   documents and answers with plain values, positions already in the client's encoding.
//! - The transport is JSON-RPC 2.0 over any byte streams, [`serve`], or stdin and stdout,
//!   [`serve_stdio`], which `smysl lsp-server` calls.
//!
//! The server never writes a store or a library and takes no lock (§2, G4).

#![forbid(unsafe_code)]

pub mod analysis;
mod transport;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// What [`serve`] runs with. `#[non_exhaustive]`, so it is built from `Default` and the `with_`
/// methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LspConfig {
    /// A library or directory whose records resolve cross-file references (LS-P4).
    pub workspace_root: Option<PathBuf>,
    /// How long after the last of a burst of changes a document is checked, in ms.
    pub check_debounce_ms: u64,
    /// Where the server logs; stderr when absent. stdout is the protocol's.
    pub log_file: Option<PathBuf>,
}

impl Default for LspConfig {
    fn default() -> LspConfig {
        LspConfig {
            workspace_root: None,
            check_debounce_ms: 150,
            log_file: None,
        }
    }
}

impl LspConfig {
    pub fn with_workspace_root(mut self, root: impl Into<PathBuf>) -> LspConfig {
        self.workspace_root = Some(root.into());
        self
    }

    pub fn with_check_debounce_ms(mut self, ms: u64) -> LspConfig {
        self.check_debounce_ms = ms;
        self
    }

    pub fn with_log_file(mut self, path: impl Into<PathBuf>) -> LspConfig {
        self.log_file = Some(path.into());
        self
    }
}

/// Why the server stopped other than by `shutdown` and `exit`.
#[derive(Debug)]
#[non_exhaustive]
pub enum LspError {
    /// The client broke the protocol: a bad frame, `exit` before `shutdown`, or a stream
    /// closed early.
    Transport(String),
    Io(std::io::Error),
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LspError::Transport(s) => write!(f, "{s}"),
            LspError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LspError {}

impl From<std::io::Error> for LspError {
    fn from(e: std::io::Error) -> LspError {
        LspError::Io(e)
    }
}

/// Serve the protocol over `input` and `output` until the client's `exit`. `Ok` after
/// `shutdown` then `exit`; an error for `exit` without `shutdown`, a broken frame, or input
/// ending first.
pub fn serve<R, W>(input: R, output: W, config: LspConfig) -> Result<(), LspError>
where
    R: BufRead + Send + 'static,
    W: Write,
{
    transport::serve(input, output, config)
}

/// [`serve`] over stdin and stdout.
pub fn serve_stdio(config: LspConfig) -> Result<(), LspError> {
    let stdout = std::io::stdout();
    serve(BufReader::new(std::io::stdin()), stdout.lock(), config)
}
