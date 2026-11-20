//! Errors, and a refusal's class read from its text (RFC SMYSL-4 §8).
//!
//! Bund2 keeps a refusal's `SMnn` code to itself. What reaches the output is
//! `Error: <WORD>: <text>`, and the text follows RFC-0014 §S8's template for its code, so the
//! code is recovered from the template. Where a template is the catch-all `<what> <reason>`
//! (SM03), the class is the caller's script or arguments.

use std::path::PathBuf;

/// What went wrong with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BundError {
    BinaryNotFound {
        searched: String,
    },
    BinaryMissingSmyslFeature {
        path: PathBuf,
        version: String,
    },
    /// A `bund2` older than [`crate::MIN_BUND2`], or one whose `--version` this bridge cannot read.
    BinaryUnsupported {
        path: PathBuf,
        version: String,
    },
    /// The gate refused a word, or the sandbox stopped an operation.
    PolicyViolation {
        reason: String,
        token: Option<String>,
    },
    ScriptTooLarge {
        bytes: usize,
        max: usize,
    },
    /// `Trust::Sandboxed` on a platform without the sandbox (§5.5).
    SandboxUnavailable {
        platform: String,
    },
    TimeoutExceeded {
        max_wall_ms: u64,
    },
    OutputTooLarge {
        bytes: usize,
        max: usize,
    },
    /// The script, or what it passed a word, was wrong: SM00, SM03, SM05, SM11, E50, or a
    /// script the gate could not read. `code` is the code the text's template implies, or empty.
    InvalidScriptOrArgs {
        code: String,
        message: String,
    },
    /// SMYSL refused: SM01, SM02, SM07, SM08, SM09, SM12.
    SmyslRefusal {
        code: String,
        message: String,
    },
    /// SM04.
    SqQueryRefusal {
        diagnostics: String,
    },
    /// SM10.
    LambdaFailure {
        message: String,
    },
    /// E30: `kb.inferences`' cap was reached.
    InferenceCapReached {
        message: String,
    },
    /// A Bund2 error that is not a smysl refusal: an unknown word, a type error in the script.
    ScriptFailed {
        message: String,
    },
    ProcessFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    NormalizationError {
        message: String,
    },
    IoError {
        message: String,
    },
}

impl std::fmt::Display for BundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundError::BinaryNotFound { searched } => write!(f, "no bund2 found ({searched})"),
            BundError::BinaryMissingSmyslFeature { path, version } => write!(
                f,
                "{} ({version}) was built without the smysl feature; build it with \
                 `cargo build -p bund2-cli --features smysl`",
                path.display()
            ),
            BundError::BinaryUnsupported { path, version } => {
                let (a, b, c) = crate::MIN_BUND2;
                write!(
                    f,
                    "{} reports `{version}`; this bridge needs bund2 {a}.{b}.{c} or later",
                    path.display()
                )
            }
            BundError::PolicyViolation {
                reason,
                token: Some(t),
            } => write!(f, "refused `{t}`: {reason}"),
            BundError::PolicyViolation {
                reason,
                token: None,
            } => write!(f, "refused: {reason}"),
            BundError::ScriptTooLarge { bytes, max } => {
                write!(f, "the script is {bytes} bytes; the limit is {max}")
            }
            BundError::SandboxUnavailable { platform } => write!(
                f,
                "no sandbox on {platform}; only a trusted script runs here"
            ),
            BundError::TimeoutExceeded { max_wall_ms } => {
                write!(f, "stopped after {max_wall_ms} ms")
            }
            BundError::OutputTooLarge { bytes, max } => {
                write!(f, "{bytes} bytes of output; the limit is {max}")
            }
            BundError::InvalidScriptOrArgs { code, message }
            | BundError::SmyslRefusal { code, message } => {
                if code.is_empty() {
                    write!(f, "{message}")
                } else {
                    write!(f, "{code}: {message}")
                }
            }
            BundError::SqQueryRefusal { diagnostics } => write!(f, "SM04: {diagnostics}"),
            BundError::LambdaFailure { message } => write!(f, "SM10: {message}"),
            BundError::InferenceCapReached { message }
            | BundError::ScriptFailed { message }
            | BundError::NormalizationError { message }
            | BundError::IoError { message } => write!(f, "{message}"),
            BundError::ProcessFailed { exit_code, stderr } => {
                write!(f, "bund2 failed (exit {exit_code:?}): {stderr}")
            }
        }
    }
}

impl std::error::Error for BundError {}

/// The class of `Error: <message>`, the first line of a failure under `--report trace`.
pub fn classify(message: &str) -> BundError {
    let message = message.trim();
    // The two house texts, unprefixed.
    if message.contains("functions disabled with --noio") {
        // SM06. The bridge never passes `--noio`, so this is a defect, not the script's.
        return BundError::ProcessFailed {
            exit_code: None,
            stderr: message.to_string(),
        };
    }
    if message.starts_with("Stack is too shallow for inline") {
        return invalid("SM11", message);
    }
    if message.contains("inference cap of") && message.ends_with("reached") {
        return BundError::InferenceCapReached {
            message: message.to_string(),
        };
    }
    if message.contains("is askable, and this query has no ask lambda") {
        return invalid("E50", message);
    }
    let Some((word, text)) = message.split_once(": ") else {
        return BundError::ScriptFailed {
            message: message.to_string(),
        };
    };
    if word != "SMYSL" && !word.starts_with("SMYSL.") {
        return BundError::ScriptFailed {
            message: message.to_string(),
        };
    }
    // §6.1's model words are refused by the gate, so reaching one means the gate was passed
    // by a name built at run time: SM13 or not, it is a policy matter.
    if matches!(
        word,
        "SMYSL.INGEST" | "SMYSL.ANSWER" | "SMYSL.INGEST.TEXT" | "SMYSL.COMMIT"
    ) {
        return BundError::PolicyViolation {
            reason: format!("the model layer ran: {text}"),
            token: Some(word.to_lowercase()),
        };
    }
    let refusal = |code: &str| BundError::SmyslRefusal {
        code: code.to_string(),
        message: message.to_string(),
    };
    if text.ends_with("parameter must be a SMYSL handle") {
        invalid("SM00", message)
    } else if text.starts_with("no SMYSL library at ") || text.starts_with("cannot reach ") {
        refusal("SM01")
    } else if text.starts_with("SMYSL refused it: ") {
        refusal("SM02")
    } else if let Some(d) = text.strip_prefix("the query was refused: ") {
        BundError::SqQueryRefusal {
            diagnostics: d.to_string(),
        }
    } else if text.starts_with("unknown ") && text.contains(": ") {
        invalid("SM05", message)
    } else if text.starts_with("the library is locked")
        || text.starts_with("the library's shards changed while locking")
    {
        refusal("SM07")
    } else if text.starts_with("label ")
        && (text.contains(" binds ") || text.contains(" is bound to "))
    {
        refusal("SM08")
    } else if text.starts_with("retraction refused: ") {
        refusal("SM09")
    } else if let Some(m) = text.strip_prefix("lambda returns: ") {
        BundError::LambdaFailure {
            message: m.to_string(),
        }
    } else if text.starts_with("the library's view allows no gist") {
        refusal("SM12")
    } else {
        // SM03's `<what> <reason>`: the script, or what it passed.
        invalid("SM03", message)
    }
}

fn invalid(code: &str, message: &str) -> BundError {
    BundError::InvalidScriptOrArgs {
        code: code.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: &BundError) -> (&'static str, String) {
        match e {
            BundError::InvalidScriptOrArgs { code, .. } => ("invalid", code.clone()),
            BundError::SmyslRefusal { code, .. } => ("refusal", code.clone()),
            BundError::SqQueryRefusal { .. } => ("sq", String::new()),
            BundError::LambdaFailure { .. } => ("lambda", String::new()),
            BundError::InferenceCapReached { .. } => ("cap", String::new()),
            BundError::ScriptFailed { .. } => ("script", String::new()),
            BundError::ProcessFailed { .. } => ("process", String::new()),
            BundError::PolicyViolation { .. } => ("policy", String::new()),
            _ => ("other", String::new()),
        }
    }

    /// One text per template of RFC-0014 §S8 and RFC-0013, as Bund2 writes it.
    #[test]
    fn each_template_has_its_class() {
        let cases: &[(&str, (&str, &str))] = &[
            (
                "SMYSL.GET: #1 parameter must be a SMYSL handle",
                ("invalid", "SM00"),
            ),
            ("SMYSL.OPEN: no SMYSL library at /x", ("refusal", "SM01")),
            (
                "SMYSL.OPEN: cannot reach /x: permission denied",
                ("refusal", "SM01"),
            ),
            (
                "SMYSL.QUERY: SMYSL refused it: SMY-E402: bad",
                ("refusal", "SM02"),
            ),
            ("SMYSL.GET: uid b3:x is not a uid", ("invalid", "SM03")),
            (
                "SMYSL.QUERY: the query was refused: SMY-E461 match is not available",
                ("sq", ""),
            ),
            (
                "SMYSL.TRACE: unknown trace kind: sideways",
                ("invalid", "SM05"),
            ),
            ("bund SMYSL functions disabled with --noio", ("process", "")),
            (
                "SMYSL.KB: the library is locked: held by 42",
                ("refusal", "SM07"),
            ),
            ("SMYSL.GET: label alpha binds 2 units", ("refusal", "SM08")),
            ("SMYSL: lambda returns: oops", ("lambda", "")),
            (
                "Stack is too shallow for inline smysl.get",
                ("invalid", "SM11"),
            ),
            (
                "SMYSL.DOSSIER: the library's view allows no gist of 40 tokens",
                ("refusal", "SM12"),
            ),
            ("SMYSL.ANSWER: provider refused", ("policy", "")),
            ("KB.SATURATE: inference cap of 5 reached", ("cap", "")),
            (
                "KB.QUERY: p/0 is askable, and this query has no ask lambda",
                ("invalid", "E50"),
            ),
            (
                "i(nosuchword) for stack returned: Inline nosuchword not registered",
                ("script", ""),
            ),
        ];
        for (text, (class, c)) in cases {
            let got = code(&classify(text));
            assert_eq!((got.0, got.1.as_str()), (*class, *c), "{text}");
        }
    }
}
