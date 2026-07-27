//! Provider configuration (§7.3 `.smysl/config.hjson`, §29).
//!
//! Parsed with the kernel's own HJSON reader, so a project has one configuration syntax
//! rather than two.
//!
//! **Keys are never here.** Only `api_key_env` or `api_key_cmd` is recorded, so a config
//! file is safe to commit and a bundle never carries a secret (§29). `ProviderConfig` has
//! no field a key could be written into, which is a stronger guarantee than a rule saying
//! not to.

use std::collections::BTreeMap;

use smysl_core::error::ProviderError;
use smysl_core::surface::hjson::parse_object_prefix;

use crate::{ProviderId, StructuredMode, Task};

/// How to reach one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProviderConfig {
    pub id: ProviderId,
    /// Which mapper drives it: `ollama`, `anthropic`, and so on.
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    pub context_window: usize,
    pub max_output: usize,
    pub structured: StructuredMode,
    /// Environment variable holding the key. Never the key.
    pub api_key_env: Option<String>,
    /// Command that prints the key. Never the key.
    pub api_key_cmd: Option<String>,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl ProviderConfig {
    pub fn new(id: ProviderId, kind: impl Into<String>) -> ProviderConfig {
        ProviderConfig {
            id,
            kind: kind.into(),
            endpoint: String::new(),
            model: String::new(),
            context_window: 4096,
            max_output: 1024,
            structured: StructuredMode::None,
            api_key_env: None,
            api_key_cmd: None,
            timeout_secs: 120,
        }
    }

    /// Whether this provider keeps everything on the machine.
    ///
    /// Decided from the endpoint rather than declared, because a configuration that could
    /// *claim* to be local would make `--offline` a promise the config file makes to
    /// itself. A loopback address is a fact.
    pub fn is_local(&self) -> bool {
        let host = self
            .endpoint
            .split("//")
            .nth(1)
            .unwrap_or(&self.endpoint)
            .split('/')
            .next()
            .unwrap_or_default()
            .rsplit('@')
            .next()
            .unwrap_or_default();
        // An IPv6 literal is bracketed, so the port separator is only the colon *after*
        // the closing bracket. Splitting on the first colon would turn `[::1]:11434` into
        // `[`, and a local endpoint would read as a remote one.
        let host = match host.strip_prefix('[') {
            Some(rest) => rest.split(']').next().unwrap_or_default(),
            None => host.split(':').next().unwrap_or_default(),
        };
        matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
            || host.ends_with(".localhost")
    }
}

/// The whole provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Config {
    pub providers: BTreeMap<ProviderId, ProviderConfig>,
    pub routing: BTreeMap<Task, ProviderId>,
    pub fallback: Vec<ProviderId>,
}

impl Config {
    /// Parse `.smysl/config.hjson`.
    pub fn load(src: &str) -> Result<Config, ProviderError> {
        let brace = src.find('{').unwrap_or(0);
        let obj = parse_object_prefix(&src[brace..], 0)
            .map_err(|e| ProviderError::Malformed(format!("config: {e}")))?
            .value;

        let mut cfg = Config::default();

        if let Some(ps) = obj.get("providers").and_then(|v| v.value.as_object()) {
            for (name, body) in ps.iter() {
                let id = ProviderId::new(name.value.clone()).ok_or_else(|| {
                    ProviderError::Malformed(format!("`{}` is not a provider id", name.value))
                })?;
                let o = body.value.as_object().ok_or_else(|| {
                    ProviderError::Malformed(format!("provider {id} is not an object"))
                })?;

                let kind = o
                    .get("kind")
                    .and_then(|v| v.value.as_str())
                    .unwrap_or(id.as_str())
                    .to_string();
                let mut p = ProviderConfig::new(id.clone(), kind);

                if let Some(v) = o.get("endpoint").and_then(|v| v.value.as_str()) {
                    p.endpoint = v.to_string();
                }
                if let Some(v) = o.get("model").and_then(|v| v.value.as_str()) {
                    p.model = v.to_string();
                }
                if let Some(v) = o.get("context_window").and_then(|v| v.value.as_int()) {
                    p.context_window = v.max(0) as usize;
                }
                if let Some(v) = o.get("max_output").and_then(|v| v.value.as_int()) {
                    p.max_output = v.max(0) as usize;
                }
                if let Some(v) = o.get("timeout_secs").and_then(|v| v.value.as_int()) {
                    p.timeout_secs = v.max(1) as u64;
                }
                if let Some(v) = o.get("structured").and_then(|v| v.value.as_str()) {
                    p.structured = structured(v).ok_or_else(|| {
                        ProviderError::Malformed(format!("`{v}` is not a structured mode"))
                    })?;
                }
                if let Some(v) = o.get("api_key_env").and_then(|v| v.value.as_str()) {
                    p.api_key_env = Some(v.to_string());
                }
                if let Some(v) = o.get("api_key_cmd").and_then(|v| v.value.as_str()) {
                    p.api_key_cmd = Some(v.to_string());
                }
                // A key in the config would end up in version control, so it is a hard
                // error rather than a warning that a hurried reader would scroll past.
                for forbidden in ["api_key", "key", "token", "secret", "password"] {
                    if o.contains(forbidden) {
                        return Err(ProviderError::Malformed(format!(
                            "provider {id} has a `{forbidden}` field; use api_key_env or \
                             api_key_cmd - a config file must be safe to commit"
                        )));
                    }
                }

                cfg.providers.insert(id, p);
            }
        }

        if let Some(r) = obj.get("routing").and_then(|v| v.value.as_object()) {
            for (task, target) in r.iter() {
                let t = Task::parse(&task.value).ok_or_else(|| {
                    ProviderError::Malformed(format!("`{}` is not a task", task.value))
                })?;
                let name = target.value.as_str().unwrap_or_default();
                let id = ProviderId::new(name).ok_or_else(|| {
                    ProviderError::Malformed(format!("`{name}` is not a provider id"))
                })?;
                cfg.routing.insert(t, id);
            }
        }

        if let Some(f) = obj.get("fallback").and_then(|v| v.value.as_array()) {
            for item in f {
                let name = item.value.as_str().unwrap_or_default();
                let id = ProviderId::new(name).ok_or_else(|| {
                    ProviderError::Malformed(format!("`{name}` is not a provider id"))
                })?;
                cfg.fallback.push(id);
            }
        }

        cfg.validate()?;
        Ok(cfg)
    }

    /// Every route and fallback must name a configured provider.
    ///
    /// Checked at load, so a typo in a task name surfaces before a model call rather than
    /// during one.
    pub fn validate(&self) -> Result<(), ProviderError> {
        for (task, id) in &self.routing {
            if !self.providers.contains_key(id) {
                return Err(ProviderError::Malformed(format!(
                    "routing sends {task} to `{id}`, which is not configured"
                )));
            }
        }
        for id in &self.fallback {
            if !self.providers.contains_key(id) {
                return Err(ProviderError::Malformed(format!(
                    "fallback names `{id}`, which is not configured"
                )));
            }
        }
        Ok(())
    }

    /// The default configuration: one local Ollama, everything routed to it.
    ///
    /// A default that reached a hosted provider would mean a first run egressing content
    /// nobody asked to send.
    pub fn local_default() -> Config {
        let id = ProviderId::new("ollama").expect("a valid literal");
        let mut p = ProviderConfig::new(id.clone(), "ollama");
        p.endpoint = "http://127.0.0.1:11434".into();
        p.model = "llama3.2".into();
        p.context_window = 8192;
        p.max_output = 2048;
        p.structured = StructuredMode::JsonSchema;

        let mut cfg = Config {
            providers: BTreeMap::from([(id.clone(), p)]),
            routing: BTreeMap::new(),
            fallback: vec![id.clone()],
        };
        for &t in Task::ALL {
            cfg.routing.insert(t, id.clone());
        }
        cfg
    }

    /// Where a project's configuration lives, relative to its root.
    pub const PATH: &'static str = ".smysl/config.hjson";
}

fn structured(s: &str) -> Option<StructuredMode> {
    match s {
        "json-schema" => Some(StructuredMode::JsonSchema),
        "tool-force" => Some(StructuredMode::ToolForce),
        "json-mode" => Some(StructuredMode::JsonMode),
        "grammar" => Some(StructuredMode::Grammar),
        "none" => Some(StructuredMode::None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  providers: {
    ollama: {
      kind: ollama
      endpoint: "http://127.0.0.1:11434"
      model: "llama3.2"
      context_window: 8192
      max_output: 2048
      structured: json-schema
    }
    remote: {
      kind: anthropic
      endpoint: "https://api.anthropic.com"
      model: "claude-sonnet-5"
      api_key_env: ANTHROPIC_API_KEY
      structured: tool-force
    }
  }
  routing: {
    content-ingest: ollama
    relation-extraction: remote
  }
  fallback: [ollama]
}"#;

    #[test]
    fn a_configuration_round_trips_its_fields() {
        let c = Config::load(SAMPLE).unwrap();
        assert_eq!(c.providers.len(), 2);

        let o = &c.providers[&ProviderId::new("ollama").unwrap()];
        assert_eq!(o.kind, "ollama");
        assert_eq!(o.model, "llama3.2");
        assert_eq!(o.context_window, 8192);
        assert_eq!(o.structured, StructuredMode::JsonSchema);

        let r = &c.providers[&ProviderId::new("remote").unwrap()];
        assert_eq!(r.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(
            c.routing[&Task::RelationExtraction],
            ProviderId::new("remote").unwrap()
        );
        assert_eq!(c.fallback, vec![ProviderId::new("ollama").unwrap()]);
    }

    /// §29: a config file must be safe to commit. A key in one is a hard error rather than
    /// a warning a hurried reader would scroll past.
    #[test]
    fn a_key_in_the_config_is_refused() {
        for field in ["api_key", "key", "token", "secret", "password"] {
            let src = format!("{{ providers: {{ p: {{ {field}: \"sk-live-abc\" }} }} }}");
            let e = Config::load(&src).expect_err("a key must not be storable");
            assert!(e.to_string().contains("safe to commit"), "{field}: {e}");
        }
    }

    #[test]
    fn there_is_no_field_a_key_could_be_written_into() {
        let c = ProviderConfig::new(ProviderId::new("p").unwrap(), "ollama");
        // The struct is the guarantee: only indirection, never a secret.
        assert!(c.api_key_env.is_none() && c.api_key_cmd.is_none());
    }

    /// Locality is decided from the endpoint, not declared: a config that could claim to
    /// be local would make `--offline` a promise the file makes to itself.
    #[test]
    fn locality_is_read_off_the_endpoint() {
        let mut p = ProviderConfig::new(ProviderId::new("p").unwrap(), "ollama");
        for local in [
            "http://127.0.0.1:11434",
            "http://localhost:11434",
            "http://[::1]:11434",
            "http://0.0.0.0:8080",
            "http://ollama.localhost/api",
        ] {
            p.endpoint = local.into();
            assert!(p.is_local(), "{local}");
        }
        for remote in [
            "https://api.anthropic.com",
            "http://192.168.1.4:11434",
            "https://user@evil.example/127.0.0.1",
            "https://127.0.0.1.evil.example",
        ] {
            p.endpoint = remote.into();
            assert!(!p.is_local(), "{remote}");
        }
    }

    /// A typo in a task name must surface before a model call rather than during one.
    #[test]
    fn routing_to_an_unconfigured_provider_is_refused_at_load() {
        let src = "{ providers: { a: { endpoint: x } }, routing: { content-ingest: b } }";
        let e = Config::load(src).unwrap_err();
        assert!(e.to_string().contains("not configured"), "{e}");
    }

    #[test]
    fn a_fallback_to_an_unconfigured_provider_is_refused_at_load() {
        let src = "{ providers: { a: { endpoint: x } }, fallback: [b] }";
        assert!(Config::load(src).is_err());
    }

    #[test]
    fn an_unknown_task_name_is_refused_rather_than_ignored() {
        let src = "{ providers: { a: { endpoint: x } }, routing: { summarise: a } }";
        assert!(Config::load(src).is_err());
    }

    #[test]
    fn an_unknown_structured_mode_is_refused() {
        let src = "{ providers: { a: { structured: telepathy } } }";
        assert!(Config::load(src).is_err());
    }

    #[test]
    fn malformed_source_is_an_error_not_a_panic() {
        assert!(Config::load("{ providers: ").is_err());
        assert!(Config::load("").is_err());
    }

    /// A default that reached a hosted provider would mean a first run egressing content
    /// nobody asked to send.
    #[test]
    fn the_default_configuration_is_entirely_local() {
        let c = Config::local_default();
        assert!(c.validate().is_ok());
        for p in c.providers.values() {
            assert!(p.is_local(), "{} is not local", p.id);
        }
        for &t in Task::ALL {
            assert!(c.routing.contains_key(&t), "{t} is unrouted by default");
        }
    }

    #[test]
    fn an_empty_configuration_is_valid_and_routes_nothing() {
        let c = Config::load("{}").unwrap();
        assert!(c.providers.is_empty());
        assert!(c.routing.is_empty());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn the_config_path_is_the_documented_one() {
        assert_eq!(Config::PATH, ".smysl/config.hjson");
    }
}
