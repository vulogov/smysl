//! Credential resolution (§29).
//!
//! Keys are never stored in configuration or in the store. A `ProviderConfig` names an
//! environment variable or a command; this module turns that name into a secret at the
//! moment of use and never anywhere else.
//!
//! Two properties the rest of the crate depends on:
//!
//! - **A key never reaches a `Debug` output.** `Secret` prints as `***`, so a provider
//!   struct can derive `Debug` without a config dump leaking a credential into a log.
//! - **A missing key is `Unauthorized`, not a panic.** An unconfigured provider is an
//!   ordinary thing to have in a config file, and `providers --probe` must be able to say
//!   so rather than aborting.

use std::fmt;

use smysl_core::error::ProviderError;

use crate::config::ProviderConfig;

/// A credential. Prints as `***` however it is formatted.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into().trim().to_string())
    }

    /// The only way to read it. Named so that a use site is greppable.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Resolve a provider's credential.
///
/// `api_key_env` first, then `api_key_cmd`. A provider naming neither needs none - which is
/// the Ollama case, and the reason this returns `Option` rather than failing.
pub fn resolve(cfg: &ProviderConfig) -> Result<Option<Secret>, ProviderError> {
    if let Some(var) = &cfg.api_key_env {
        return match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => Ok(Some(Secret::new(v))),
            // Naming a variable that is not set is a configuration error the caller can
            // fix, so it says which variable rather than "unauthorized".
            _ => Err(ProviderError::Malformed(format!(
                "{}: ${var} is unset or empty",
                cfg.id
            ))),
        };
    }

    if let Some(cmd) = &cfg.api_key_cmd {
        return run_key_command(cmd, cfg).map(Some);
    }

    Ok(None)
}

/// Run `api_key_cmd` and take its stdout.
///
/// Split on whitespace rather than handed to a shell: a config file is data, and passing
/// data to `sh -c` turns every config file into a script. A caller who genuinely wants a
/// pipeline can name a script.
fn run_key_command(cmd: &str, cfg: &ProviderConfig) -> Result<Secret, ProviderError> {
    let mut parts = cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| ProviderError::Malformed(format!("{}: api_key_cmd is empty", cfg.id)))?;

    let out = std::process::Command::new(program)
        .args(parts)
        .output()
        .map_err(|e| ProviderError::Malformed(format!("{}: api_key_cmd: {e}", cfg.id)))?;

    if !out.status.success() {
        // stderr is not included: a failing credential helper may print the secret it was
        // trying to fetch, and this message reaches logs.
        return Err(ProviderError::Malformed(format!(
            "{}: api_key_cmd exited {}",
            cfg.id,
            out.status.code().unwrap_or(-1)
        )));
    }

    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if key.is_empty() {
        return Err(ProviderError::Malformed(format!(
            "{}: api_key_cmd printed nothing",
            cfg.id
        )));
    }
    Ok(Secret::new(key))
}

/// The `Authorization: Bearer …` header value.
pub fn bearer(s: &Secret) -> String {
    format!("Bearer {}", s.expose())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderId;

    fn cfg() -> ProviderConfig {
        ProviderConfig::new(ProviderId::new("p").unwrap(), "openai")
    }

    /// A provider struct should be `Debug`-able without a log line leaking a credential.
    #[test]
    fn a_secret_never_prints_itself() {
        let s = Secret::new("sk-live-abcdef");
        assert_eq!(format!("{s}"), "***");
        assert_eq!(format!("{s:?}"), "***");
        assert!(!format!("{s:?} {s}").contains("abcdef"));
    }

    #[test]
    fn exposing_is_the_only_way_to_read_it() {
        assert_eq!(Secret::new("  key  ").expose(), "key");
    }

    #[test]
    fn a_provider_naming_no_credential_needs_none() {
        assert_eq!(resolve(&cfg()).unwrap(), None);
    }

    #[test]
    fn an_environment_variable_is_read_at_the_moment_of_use() {
        let var = "SMYSL_TEST_KEY_PRESENT";
        // SAFETY of a different kind: this is a test-only environment mutation, and the
        // variable name is unique to this test.
        std::env::set_var(var, "sk-from-env");
        let mut c = cfg();
        c.api_key_env = Some(var.into());
        assert_eq!(resolve(&c).unwrap().unwrap().expose(), "sk-from-env");
        std::env::remove_var(var);
    }

    /// Naming a variable that is not set is a configuration error the caller can fix, so
    /// the message says which variable.
    #[test]
    fn an_unset_variable_says_which_one() {
        let mut c = cfg();
        c.api_key_env = Some("SMYSL_TEST_KEY_DEFINITELY_UNSET".into());
        let e = resolve(&c).unwrap_err();
        assert!(
            e.to_string().contains("SMYSL_TEST_KEY_DEFINITELY_UNSET"),
            "{e}"
        );
    }

    #[test]
    fn an_empty_variable_is_treated_as_unset() {
        let var = "SMYSL_TEST_KEY_EMPTY";
        std::env::set_var(var, "   ");
        let mut c = cfg();
        c.api_key_env = Some(var.into());
        assert!(resolve(&c).is_err());
        std::env::remove_var(var);
    }

    #[test]
    fn a_command_supplies_a_key_from_its_stdout() {
        let mut c = cfg();
        c.api_key_cmd = Some("echo sk-from-cmd".into());
        assert_eq!(resolve(&c).unwrap().unwrap().expose(), "sk-from-cmd");
    }

    /// A config file is data. Handing it to a shell would turn every config file into a
    /// script, so the command is split on whitespace and executed directly.
    #[test]
    fn a_command_is_not_run_through_a_shell() {
        let mut c = cfg();
        c.api_key_cmd = Some("echo one; echo two".into());
        let key = resolve(&c).unwrap().unwrap();
        // Run through a shell this would be two commands; run directly it is one `echo`
        // with three arguments.
        assert_eq!(key.expose(), "one; echo two");
    }

    #[test]
    fn a_failing_command_is_an_error_without_its_stderr() {
        let mut c = cfg();
        c.api_key_cmd = Some("false".into());
        let e = resolve(&c).unwrap_err();
        assert!(e.to_string().contains("exited"), "{e}");
    }

    #[test]
    fn a_command_that_prints_nothing_is_an_error() {
        let mut c = cfg();
        c.api_key_cmd = Some("true".into());
        assert!(resolve(&c).is_err());
    }

    #[test]
    fn a_missing_command_is_an_error_not_a_panic() {
        let mut c = cfg();
        c.api_key_cmd = Some("smysl-no-such-credential-helper".into());
        assert!(resolve(&c).is_err());
    }

    #[test]
    fn the_environment_variable_wins_over_the_command() {
        let var = "SMYSL_TEST_KEY_PRECEDENCE";
        std::env::set_var(var, "from-env");
        let mut c = cfg();
        c.api_key_env = Some(var.into());
        c.api_key_cmd = Some("echo from-cmd".into());
        assert_eq!(resolve(&c).unwrap().unwrap().expose(), "from-env");
        std::env::remove_var(var);
    }

    #[test]
    fn a_bearer_header_is_the_conventional_shape() {
        assert_eq!(bearer(&Secret::new("abc")), "Bearer abc");
    }
}
