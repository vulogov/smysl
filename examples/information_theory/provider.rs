//! A scripted model provider, so the example runs offline and gives the same answer every time.
//!
//! It extracts one claim per chat message it can see in the window, quoting the message's first
//! words so that span attachment finds the bytes. Being deterministic, it reproduces at
//! temperature 0, which is what the consensus run's `reproduced` count reports (OQ-74).

use smysl::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};

use crate::corpus::MESSAGES;

pub struct Scripted {
    id: ProviderId,
    caps: Capabilities,
}

/// The first `n` words of `text`: a quote the message certainly contains.
fn opening(text: &str, n: usize) -> String {
    text.split_whitespace()
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Provider for Scripted {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let prompt: String = req
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let units: Vec<String> = MESSAGES
            .iter()
            .filter(|(_, _, _, text, _)| prompt.contains(&opening(text, 6)))
            .map(|(id, who, _, text, _)| {
                format!(
                    r#"{{"type":"claim","label":"c/m{id}","gist":"{who} said: {}","status":"cited","quote":"{}"}}"#,
                    opening(text, 8),
                    opening(text, 6),
                )
            })
            .collect();
        Ok(Completion::new(
            format!(r#"{{"units":[{}]}}"#, units.join(",")),
            "scripted",
            Usage::reported(40, 60),
        ))
    }

    fn probe(&self) -> Result<Probe, ProviderError> {
        Ok(Probe::reachable(
            vec!["scripted".into()],
            self.caps.clone(),
            "",
        ))
    }
}

/// A registry that routes content ingest to the scripted provider.
pub fn registry() -> Registry {
    let mut caps = Capabilities::default();
    caps.offline = true;
    caps.context_window = 2048;
    caps.structured = StructuredMode::JsonSchema;
    let p = Scripted {
        id: ProviderId::new("scripted").expect("a provider id"),
        caps,
    };
    let id = p.id();
    Registry::new()
        .with_provider(Box::new(p))
        .route(Task::ContentIngest, id)
}
