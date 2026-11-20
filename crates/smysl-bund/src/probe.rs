//! What a `bund2` binary is (RFC SMYSL-4 §9).
//!
//! `bund2 --version` prints `bund2 <version> (features: a, b)` or `bund2 <version> (default
//! features)` on its first line (`bund2-cli/src/main.rs:56-58`). The `smysl` feature is named
//! there when it is compiled in.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::BundError;

/// A probed binary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BundBinaryInfo {
    pub path: PathBuf,
    /// As `--version` printed it, e.g. `0.56.0`.
    pub version: String,
    pub has_smysl: bool,
    pub has_smysl_model: bool,
}

/// Run `bund2 --version` and read it. `explicit` is used as given; otherwise `bund2` is looked
/// for on `PATH`.
pub fn probe_bund_binary(explicit: Option<&Path>) -> Result<BundBinaryInfo, BundError> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => on_path("bund2").ok_or_else(|| BundError::BinaryNotFound {
            searched: "bund2 on PATH".to_string(),
        })?,
    };
    let out = Command::new(&path)
        .arg("--version")
        .env_clear()
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BundError::BinaryNotFound {
                    searched: path.display().to_string(),
                }
            } else {
                BundError::IoError {
                    message: format!("{}: {e}", path.display()),
                }
            }
        })?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or_default();
    read_version(&path, first)
}

/// Read `--version`'s first line, and refuse what this bridge does not run.
pub fn read_version(path: &Path, line: &str) -> Result<BundBinaryInfo, BundError> {
    let unsupported = || BundError::BinaryUnsupported {
        path: path.to_path_buf(),
        version: line.to_string(),
    };
    let rest = line.strip_prefix("bund2 ").ok_or_else(unsupported)?;
    let (version, features) = rest.split_once(' ').unwrap_or((rest, ""));
    let parts: Vec<u64> = version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| unsupported())?;
    let [a, b, c] = parts[..] else {
        return Err(unsupported());
    };
    if (a, b, c) < crate::MIN_BUND2 {
        return Err(unsupported());
    }
    let named: Vec<&str> = features
        .trim()
        .strip_prefix("(features:")
        .and_then(|f| f.strip_suffix(')'))
        .map(|f| f.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let info = BundBinaryInfo {
        path: path.to_path_buf(),
        version: version.to_string(),
        has_smysl: named.contains(&"smysl"),
        has_smysl_model: named.contains(&"smysl-model"),
    };
    if !info.has_smysl {
        return Err(BundError::BinaryMissingSmyslFeature {
            path: info.path,
            version: info.version,
        });
    }
    Ok(info)
}

fn on_path(name: &str) -> Option<PathBuf> {
    // The bridge's one read of the environment: where to find the binary when none is given.
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(line: &str) -> Result<BundBinaryInfo, BundError> {
        read_version(Path::new("/x/bund2"), line)
    }

    #[test]
    fn a_smysl_build() {
        let i = read("bund2 0.56.0 (features: smysl)").unwrap();
        assert_eq!(
            (i.version.as_str(), i.has_smysl, i.has_smysl_model),
            ("0.56.0", true, false)
        );
        let i = read("bund2 0.57.1 (features: jit, smysl, smysl-model, smysl-ollama)").unwrap();
        assert!(i.has_smysl && i.has_smysl_model);
    }

    #[test]
    fn without_smysl() {
        assert!(matches!(
            read("bund2 0.56.0 (default features)"),
            Err(BundError::BinaryMissingSmyslFeature { .. })
        ));
        assert!(matches!(
            read("bund2 0.56.0 (features: jit)"),
            Err(BundError::BinaryMissingSmyslFeature { .. })
        ));
    }

    #[test]
    fn too_old_or_unreadable() {
        assert!(matches!(
            read("bund2 0.55.9 (features: smysl)"),
            Err(BundError::BinaryUnsupported { .. })
        ));
        assert!(matches!(
            read("bund 0.56.0"),
            Err(BundError::BinaryUnsupported { .. })
        ));
        assert!(matches!(
            read("bund2 x.y"),
            Err(BundError::BinaryUnsupported { .. })
        ));
    }
}
