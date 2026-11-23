//! Whether the request the current task is handling reached an upstream.
//!
//! A request LocalRouter answers itself (a rate-limit refusal, a firewall
//! denial, a validation error) never reached a provider: it is not a request
//! against any limit and must not be written as a failed request into the
//! metrics the limits read. The server scopes a probe per request
//! ([`with_upstream_probe`]); the router and the provider HTTP client mark it
//! when they dispatch ([`mark_upstream_dispatch`]). [`refused_locally`] is
//! true only when a probe is in scope and nothing was dispatched, so code
//! outside a probe keeps counting every failure.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

tokio::task_local! {
    static UPSTREAM_PROBE: Option<Arc<AtomicBool>>;
}

/// Run `fut` with a fresh probe in scope.
pub async fn with_upstream_probe<F: std::future::Future>(fut: F) -> F::Output {
    UPSTREAM_PROBE
        .scope(Some(Arc::new(AtomicBool::new(false))), fut)
        .await
}

/// Record that the current request is being sent to an upstream provider.
pub fn mark_upstream_dispatch() {
    let _ = UPSTREAM_PROBE.try_with(|probe| {
        if let Some(probe) = probe {
            probe.store(true, Ordering::SeqCst);
        }
    });
}

/// True when a probe is in scope and the request never reached an upstream.
pub fn refused_locally() -> bool {
    UPSTREAM_PROBE
        .try_with(|probe| {
            probe
                .as_ref()
                .is_some_and(|probe| !probe.load(Ordering::SeqCst))
        })
        .unwrap_or(false)
}

/// The probe in scope, to carry into a spawned task.
pub(crate) fn current_probe() -> Option<Arc<AtomicBool>> {
    UPSTREAM_PROBE.try_with(Clone::clone).ok().flatten()
}

/// Run `fut` with `probe` (from [`current_probe`]) in scope.
pub(crate) async fn with_probe<F: std::future::Future>(
    probe: Option<Arc<AtomicBool>>,
    fut: F,
) -> F::Output {
    UPSTREAM_PROBE.scope(probe, fut).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_an_undispatched_probe_is_a_local_refusal() {
        assert!(!refused_locally(), "no probe: count as before");
        mark_upstream_dispatch(); // no probe: no-op
        with_upstream_probe(async {
            assert!(refused_locally());
            mark_upstream_dispatch();
            assert!(!refused_locally());
        })
        .await;
        with_upstream_probe(async {
            // A spawned task sees and marks the request's probe.
            crate::spawn_traced(async { mark_upstream_dispatch() })
                .await
                .unwrap();
            assert!(!refused_locally());
        })
        .await;
    }
}
