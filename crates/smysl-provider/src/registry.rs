//! Routing and fallback (§21.3).
//!
//! Two rules, and both are about refusing to be clever:
//!
//! **Offline is checked before anything is sent.** `offline == true` and
//! `caps().offline == false` is `OfflineViolation` and exit 7, decided from configuration
//! alone. No socket is opened, no DNS lookup is made, and the check does not depend on
//! whether the provider happens to be reachable.
//!
//! **Fallback fires only on `Unreachable`.** Falling back on `Unauthorized`,
//! `ContextExceeded` or `Malformed` would hide a configuration error behind a different
//! model - the caller would get an answer, from somewhere they did not choose, and never
//! learn their key was wrong.

use std::collections::BTreeMap;

use smysl_core::error::ProviderError;

use crate::http::is_retryable;
use crate::{Completion, Probe, Provider, ProviderId, Request, Task};

/// Which providers exist, what each task routes to, and what to try when one is down.
#[derive(Default)]
pub struct Registry {
    providers: BTreeMap<ProviderId, Box<dyn Provider>>,
    tasks: BTreeMap<Task, ProviderId>,
    fallback: Vec<ProviderId>,
    offline: bool,
}

/// What actually happened, for the ledger and for `--verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Routed {
    pub completion: Completion,
    /// Which provider answered - not necessarily the one the task routes to.
    pub provider: ProviderId,
    /// Providers tried and found unreachable, in order.
    pub skipped: Vec<ProviderId>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    pub fn with_provider(mut self, p: Box<dyn Provider>) -> Registry {
        self.providers.insert(p.id(), p);
        self
    }

    pub fn route(mut self, t: Task, id: ProviderId) -> Registry {
        self.tasks.insert(t, id);
        self
    }

    pub fn with_fallback(mut self, ids: impl IntoIterator<Item = ProviderId>) -> Registry {
        self.fallback = ids.into_iter().collect();
        self
    }

    /// `--offline`: hard-fail rather than fall back to a hosted provider (§29).
    pub fn offline(mut self, yes: bool) -> Registry {
        self.offline = yes;
        self
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    pub fn get(&self, id: &ProviderId) -> Option<&dyn Provider> {
        self.providers.get(id).map(|b| b.as_ref())
    }

    pub fn ids(&self) -> Vec<ProviderId> {
        self.providers.keys().cloned().collect()
    }

    pub fn routing(&self) -> &BTreeMap<Task, ProviderId> {
        &self.tasks
    }

    pub fn fallback_chain(&self) -> &[ProviderId] {
        &self.fallback
    }

    /// The provider a task routes to (§21.3).
    ///
    /// Refuses on `--offline` before any I/O, so the refusal costs nothing and cannot
    /// depend on network conditions.
    pub fn for_task(&self, t: Task) -> Result<&dyn Provider, ProviderError> {
        let id = self
            .tasks
            .get(&t)
            .ok_or_else(|| ProviderError::Malformed(format!("no provider routed for {t}")))?;
        let p = self
            .providers
            .get(id)
            .ok_or_else(|| ProviderError::Malformed(format!("`{id}` is not configured")))?
            .as_ref();
        self.admit(p)?;
        Ok(p)
    }

    /// Whether `--offline` permits this provider. Configuration only; nothing is sent.
    fn admit(&self, p: &dyn Provider) -> Result<(), ProviderError> {
        if self.offline && !p.caps().offline {
            return Err(ProviderError::OfflineViolation);
        }
        Ok(())
    }

    /// Which providers would be tried for a task, in order.
    ///
    /// The routed provider first, then the fallback chain with it removed - trying the same
    /// provider twice would double the wait for no new information.
    pub fn chain(&self, t: Task) -> Vec<ProviderId> {
        let mut out = Vec::new();
        if let Some(id) = self.tasks.get(&t) {
            out.push(id.clone());
        }
        for id in &self.fallback {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
        out.retain(|id| self.providers.contains_key(id));
        out
    }

    /// Complete a task, falling back only on `Unreachable`.
    pub fn complete(&self, t: Task, req: &Request) -> Result<Routed, ProviderError> {
        let chain = self.chain(t);
        if chain.is_empty() {
            return Err(ProviderError::Malformed(format!(
                "no provider routed for {t}"
            )));
        }

        let mut skipped = Vec::new();
        let mut last = ProviderError::Unreachable;

        for id in &chain {
            let Some(p) = self.providers.get(id) else {
                continue;
            };
            // Offline is per provider, not per chain: a local provider stays usable when
            // the hosted one it would fall back to is forbidden.
            if let Err(e) = self.admit(p.as_ref()) {
                skipped.push(id.clone());
                last = e;
                continue;
            }
            match p.complete(req) {
                Ok(completion) => {
                    return Ok(Routed {
                        completion,
                        provider: id.clone(),
                        skipped,
                    })
                }
                // The one case worth trying somewhere else, decided by the kernel's own
                // predicate rather than by a second copy of the rule here. Anything else
                // is a fact about the request, and a different model produces the same
                // fact.
                Err(e) if e.is_fallback_eligible() => {
                    skipped.push(id.clone());
                    last = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Probe every provider. **No content egress.**
    pub fn probe_all(&self) -> Vec<(ProviderId, Result<Probe, ProviderError>)> {
        self.providers
            .iter()
            .map(|(id, p)| {
                let r = match self.admit(p.as_ref()) {
                    // Probing a forbidden provider would be a network call `--offline`
                    // exists to prevent, however harmless the payload.
                    Err(e) => Err(e),
                    Ok(()) => p.probe(),
                };
                (id.clone(), r)
            })
            .collect()
    }

    /// What would leave the machine under current routing (§29, `providers --tasks`).
    pub fn egress_report(&self) -> Vec<Egress> {
        Task::ALL
            .iter()
            .map(|&task| {
                let provider = self.tasks.get(&task).cloned();
                let local = provider
                    .as_ref()
                    .and_then(|id| self.providers.get(id))
                    .map(|p| p.caps().offline);
                Egress {
                    task,
                    provider,
                    leaves_machine: match local {
                        Some(true) => false,
                        Some(false) => task.egresses_content(),
                        // An unrouted task cannot run at all, so nothing leaves - but it
                        // is worth saying which, because a caller reading this table is
                        // deciding what is safe to run.
                        None => false,
                    },
                    routed: local.is_some(),
                }
            })
            .collect()
    }

    /// Whether the retry policy would try this error again. Re-exported here so a caller
    /// deciding what to report does not have to reach into `http`.
    pub fn would_retry(e: &ProviderError) -> bool {
        is_retryable(e)
    }
}

/// One row of `providers --tasks`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Egress {
    pub task: Task,
    pub provider: Option<ProviderId>,
    pub leaves_machine: bool,
    pub routed: bool,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Capabilities, Usage};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A provider that answers from a script. Everything about routing, fallback and
    /// offline is decided before any I/O, so testing it needs no network at all - which is
    /// also the claim the gate makes.
    pub struct Mock {
        pub id: ProviderId,
        pub caps: Capabilities,
        pub answer: Result<String, ProviderError>,
        pub calls: Arc<AtomicUsize>,
    }

    impl Mock {
        pub fn new(id: &str, offline: bool) -> Mock {
            Mock {
                id: ProviderId::new(id).unwrap(),
                caps: Capabilities {
                    offline,
                    ..Capabilities::default()
                },
                answer: Ok(format!("from {id}")),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        pub fn failing(id: &str, offline: bool, e: ProviderError) -> Mock {
            Mock {
                answer: Err(e),
                ..Mock::new(id, offline)
            }
        }
    }

    impl Provider for Mock {
        fn id(&self) -> ProviderId {
            self.id.clone()
        }

        fn caps(&self) -> Capabilities {
            self.caps.clone()
        }

        fn complete(&self, _req: &Request) -> Result<Completion, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.answer {
                Ok(text) => Ok(Completion {
                    text: text.clone(),
                    model: "mock".into(),
                    usage: Usage::default(),
                    structured: false,
                }),
                Err(e) => Err(e.clone()),
            }
        }

        fn probe(&self) -> Result<Probe, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.answer {
                Ok(_) => Ok(Probe {
                    reachable: true,
                    models: vec!["mock".into()],
                    caps: Some(self.caps.clone()),
                    detail: String::new(),
                }),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn req() -> Request {
        Request::new("mock", "hello")
    }

    fn id(s: &str) -> ProviderId {
        ProviderId::new(s).unwrap()
    }

    // -- routing -------------------------------------------------------------

    #[test]
    fn a_task_reaches_the_provider_it_is_routed_to() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("local", true)))
            .with_provider(Box::new(Mock::new("hosted", false)))
            .route(Task::ContentIngest, id("hosted"));
        assert_eq!(r.for_task(Task::ContentIngest).unwrap().id(), id("hosted"));
    }

    #[test]
    fn an_unrouted_task_is_an_error_rather_than_a_guess() {
        let r = Registry::new().with_provider(Box::new(Mock::new("local", true)));
        assert!(r.for_task(Task::ContentIngest).is_err());
        assert!(r.complete(Task::ContentIngest, &req()).is_err());
    }

    #[test]
    fn routing_to_a_missing_provider_is_an_error() {
        let r = Registry::new().route(Task::Attest, id("nowhere"));
        assert!(r.for_task(Task::Attest).is_err());
    }

    // -- offline (gate) ------------------------------------------------------

    /// **The gate.** `--offline` with a hosted provider fails before any I/O: the mock
    /// counts its calls, and the count stays zero.
    #[test]
    fn offline_with_a_hosted_provider_fails_without_a_call() {
        let hosted = Mock::new("hosted", false);
        let calls = Arc::clone(&hosted.calls);
        let r = Registry::new()
            .with_provider(Box::new(hosted))
            .route(Task::ContentIngest, id("hosted"))
            .offline(true);

        assert_eq!(
            r.for_task(Task::ContentIngest).map(|_| ()).unwrap_err(),
            ProviderError::OfflineViolation
        );
        assert_eq!(
            r.complete(Task::ContentIngest, &req()).unwrap_err(),
            ProviderError::OfflineViolation
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was sent");
    }

    #[test]
    fn an_offline_violation_carries_exit_code_seven() {
        assert_eq!(
            ProviderError::OfflineViolation.exit_code(),
            smysl_core::ExitCode::Offline
        );
        assert_eq!(smysl_core::ExitCode::Offline as u8, 7);
        assert_eq!(
            ProviderError::Unreachable.exit_code(),
            smysl_core::ExitCode::Provider
        );
    }

    #[test]
    fn offline_permits_a_local_provider() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("local", true)))
            .route(Task::ContentIngest, id("local"))
            .offline(true);
        assert!(r.for_task(Task::ContentIngest).is_ok());
        assert_eq!(
            r.complete(Task::ContentIngest, &req())
                .unwrap()
                .completion
                .text,
            "from local"
        );
    }

    /// Offline is per provider, not per chain: a local provider stays usable when the
    /// hosted one it would fall back to is forbidden.
    #[test]
    fn offline_skips_a_hosted_fallback_and_keeps_a_local_one() {
        let hosted = Mock::new("hosted", false);
        let hosted_calls = Arc::clone(&hosted.calls);
        let r = Registry::new()
            .with_provider(Box::new(Mock::failing(
                "primary",
                true,
                ProviderError::Unreachable,
            )))
            .with_provider(Box::new(hosted))
            .with_provider(Box::new(Mock::new("local", true)))
            .route(Task::ContentIngest, id("primary"))
            .with_fallback([id("hosted"), id("local")])
            .offline(true);

        let out = r.complete(Task::ContentIngest, &req()).unwrap();
        assert_eq!(out.provider, id("local"));
        assert_eq!(out.skipped, vec![id("primary"), id("hosted")]);
        assert_eq!(hosted_calls.load(Ordering::SeqCst), 0, "nothing was sent");
    }

    /// Probing a forbidden provider would be exactly the network call `--offline` exists
    /// to prevent, however harmless the payload.
    #[test]
    fn offline_refuses_to_probe_a_hosted_provider() {
        let hosted = Mock::new("hosted", false);
        let calls = Arc::clone(&hosted.calls);
        let r = Registry::new()
            .with_provider(Box::new(hosted))
            .offline(true);
        let probes = r.probe_all();
        assert_eq!(probes[0].1, Err(ProviderError::OfflineViolation));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // -- fallback (gate) -----------------------------------------------------

    /// **The gate.** Fallback fires on `Unreachable`.
    #[test]
    fn fallback_fires_on_unreachable() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::failing(
                "down",
                true,
                ProviderError::Unreachable,
            )))
            .with_provider(Box::new(Mock::new("spare", true)))
            .route(Task::ContentIngest, id("down"))
            .with_fallback([id("spare")]);

        let out = r.complete(Task::ContentIngest, &req()).unwrap();
        assert_eq!(out.provider, id("spare"));
        assert_eq!(out.skipped, vec![id("down")]);
    }

    /// **The gate, other half.** Falling back on anything else would hide a configuration
    /// error behind a different model: the caller would get an answer, from somewhere they
    /// did not choose, and never learn their key was wrong.
    #[test]
    fn fallback_fires_on_nothing_else() {
        for e in [
            ProviderError::Unauthorized,
            ProviderError::ContextExceeded {
                limit: 10,
                requested: 99,
            },
            ProviderError::Malformed("bad json".into()),
            ProviderError::StructuredUnsupported,
            ProviderError::RateLimited { retry_after: None },
            ProviderError::Upstream(500, "boom".into()),
        ] {
            let spare = Mock::new("spare", true);
            let spare_calls = Arc::clone(&spare.calls);
            let r = Registry::new()
                .with_provider(Box::new(Mock::failing("primary", true, e.clone())))
                .with_provider(Box::new(spare))
                .route(Task::ContentIngest, id("primary"))
                .with_fallback([id("spare")]);

            assert_eq!(
                r.complete(Task::ContentIngest, &req()).unwrap_err(),
                e,
                "{e} should surface, not fall back"
            );
            assert_eq!(
                spare_calls.load(Ordering::SeqCst),
                0,
                "{e} reached the fallback"
            );
        }
    }

    #[test]
    fn an_exhausted_chain_reports_unreachable() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::failing(
                "a",
                true,
                ProviderError::Unreachable,
            )))
            .with_provider(Box::new(Mock::failing(
                "b",
                true,
                ProviderError::Unreachable,
            )))
            .route(Task::ContentIngest, id("a"))
            .with_fallback([id("b")]);
        assert_eq!(
            r.complete(Task::ContentIngest, &req()).unwrap_err(),
            ProviderError::Unreachable
        );
    }

    /// Trying the same provider twice would double the wait for no new information.
    #[test]
    fn the_routed_provider_is_not_tried_twice() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("only", true)))
            .route(Task::ContentIngest, id("only"))
            .with_fallback([id("only")]);
        assert_eq!(r.chain(Task::ContentIngest), vec![id("only")]);
    }

    #[test]
    fn the_chain_drops_providers_that_are_not_configured() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("real", true)))
            .route(Task::ContentIngest, id("real"))
            .with_fallback([id("ghost")]);
        assert_eq!(r.chain(Task::ContentIngest), vec![id("real")]);
    }

    #[test]
    fn a_working_provider_is_used_without_touching_the_fallback() {
        let spare = Mock::new("spare", true);
        let calls = Arc::clone(&spare.calls);
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("primary", true)))
            .with_provider(Box::new(spare))
            .route(Task::ContentIngest, id("primary"))
            .with_fallback([id("spare")]);
        assert!(r
            .complete(Task::ContentIngest, &req())
            .unwrap()
            .skipped
            .is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // -- egress reporting ----------------------------------------------------

    /// §29: `providers --tasks` reports exactly what would leave the machine.
    #[test]
    fn the_egress_report_names_what_leaves_the_machine() {
        let r = Registry::new()
            .with_provider(Box::new(Mock::new("local", true)))
            .with_provider(Box::new(Mock::new("hosted", false)))
            .route(Task::ContentIngest, id("local"))
            .route(Task::RelationExtraction, id("hosted"));

        let rows = r.egress_report();
        let row = |t: Task| rows.iter().find(|e| e.task == t).unwrap().clone();

        assert!(!row(Task::ContentIngest).leaves_machine);
        assert!(row(Task::RelationExtraction).leaves_machine);
        assert!(!row(Task::Attest).routed, "unrouted tasks say so");
        assert!(!row(Task::Attest).leaves_machine);
        assert_eq!(rows.len(), Task::ALL.len(), "every task is accounted for");
    }

    #[test]
    fn retry_advice_is_reachable_from_the_registry() {
        assert!(Registry::would_retry(&ProviderError::RateLimited {
            retry_after: None
        }));
        assert!(!Registry::would_retry(&ProviderError::Unreachable));
    }
}
