//! Server→client requests the gateway forwards to its downstream client
//! (passthrough `sampling/createMessage` and `elicitation/create`).
//!
//! The upstream's request reaches the session's request callback; the
//! gateway sends it to the client on the session's stream under a fresh id
//! (`McpGateway::client_request_sender`) and waits for the answer. This
//! module adds what the plain wait lacked:
//!
//! - **Target check.** A request is forwarded only to a client that declared
//!   the capability in `initialize` (the gateway advertises sampling and
//!   elicitation upstream whatever the client declared).
//! - **Cancellation.** An upstream `notifications/cancelled` for a forwarded
//!   request ends the wait and is passed on with the id the client saw; one
//!   that matches nothing is left to the normal forwarding.
//! - **Timeout.** When the wait times out the client is told the request is
//!   cancelled, so it can close its prompt.
//!
//! In-flight entries live only while their wait does (drop guard).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

use super::session::GatewaySession;

/// How the gateway reaches a session's client: `(session key, request)` →
/// receiver of the client's answer, or `None` without a stream.
pub(crate) type ClientRequestSender = Arc<
    dyn Fn(&str, JsonRpcRequest) -> Option<tokio::sync::oneshot::Receiver<JsonRpcResponse>>
        + Send
        + Sync,
>;

type NotificationBroadcast = Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>;

/// How long a forwarded request waits for the client.
pub(crate) const FORWARD_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a cancellation that arrived before its request is remembered.
const EARLY_CANCEL_TTL: Duration = Duration::from_secs(30);
/// Bound on remembered early cancellations (oldest dropped first).
const EARLY_CANCEL_CAP: usize = 256;

/// `(session key, server id, upstream request id)`; the id in its JSON form,
/// so `1` and `"1"` stay distinct.
type ForwardKey = (String, String, String);

#[derive(Default)]
struct Registry {
    /// Each entry with the id of the wait that registered it, so a wait
    /// that ends removes only its own entry (an upstream that reconnected
    /// may reuse a request id under the same key).
    inflight: HashMap<ForwardKey, (u64, CancellationToken)>,
    next_wait: u64,
    /// Cancellations of requests not seen yet: the transport hands the
    /// request and a cancellation right behind it to two tasks, which may
    /// run in either order.
    early: HashMap<ForwardKey, Instant>,
}

/// Requests forwarded to downstream clients and not yet answered.
#[derive(Clone, Default)]
pub(crate) struct ForwardedRequests {
    registry: Arc<parking_lot::Mutex<Registry>>,
}

/// Removes its entry when the forwarded request's wait ends.
pub(crate) struct ForwardGuard {
    registry: Arc<parking_lot::Mutex<Registry>>,
    key: ForwardKey,
    wait: u64,
}

impl Drop for ForwardGuard {
    fn drop(&mut self) {
        let mut registry = self.registry.lock();
        if registry
            .inflight
            .get(&self.key)
            .is_some_and(|(wait, _)| *wait == self.wait)
        {
            registry.inflight.remove(&self.key);
        }
    }
}

fn forward_key(session_key: &str, server_id: &str, upstream_id: &Value) -> ForwardKey {
    (
        session_key.to_string(),
        server_id.to_string(),
        upstream_id.to_string(),
    )
}

impl ForwardedRequests {
    fn register(
        &self,
        session_key: &str,
        server_id: &str,
        upstream_id: &Value,
    ) -> (ForwardGuard, CancellationToken) {
        let key = forward_key(session_key, server_id, upstream_id);
        let token = CancellationToken::new();
        let wait = {
            let mut registry = self.registry.lock();
            if registry
                .early
                .remove(&key)
                .is_some_and(|at| at.elapsed() < EARLY_CANCEL_TTL)
            {
                token.cancel();
            }
            registry.next_wait += 1;
            let wait = registry.next_wait;
            registry.inflight.insert(key.clone(), (wait, token.clone()));
            wait
        };
        (
            ForwardGuard {
                registry: self.registry.clone(),
                key,
                wait,
            },
            token,
        )
    }

    /// An upstream `notifications/cancelled`: cancel the forwarded request it
    /// names. Returns whether it named one in flight (the caller then drops
    /// the upstream's notification; the client gets its own with the id it
    /// saw). One naming nothing yet is remembered briefly, in case its
    /// request is still on the way.
    pub(crate) fn cancel_from_upstream(
        &self,
        session_key: &str,
        server_id: &str,
        notification: &JsonRpcNotification,
    ) -> bool {
        let Some(request_id) = notification
            .params
            .as_ref()
            .and_then(|p| p.get("requestId"))
        else {
            return false;
        };
        let key = forward_key(session_key, server_id, request_id);
        let mut registry = self.registry.lock();
        if let Some((_, token)) = registry.inflight.get(&key) {
            token.cancel();
            return true;
        }
        registry
            .early
            .retain(|_, at| at.elapsed() < EARLY_CANCEL_TTL);
        if registry.early.len() >= EARLY_CANCEL_CAP {
            if let Some(oldest) = registry
                .early
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(k, _)| k.clone())
            {
                registry.early.remove(&oldest);
            }
        }
        registry.early.insert(key, Instant::now());
        false
    }

    /// The session `session_key` ended: nobody can answer its forwarded
    /// requests any more, so their waits end now instead of at the timeout
    /// (the client, if it is still there, is told they are cancelled).
    pub(crate) fn cancel_session(&self, session_key: &str) {
        let mut registry = self.registry.lock();
        for ((session, _, _), (_, token)) in registry.inflight.iter() {
            if session == session_key {
                token.cancel();
            }
        }
        registry
            .early
            .retain(|(session, _, _), _| session != session_key);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.registry.lock().inflight.len()
    }

    #[cfg(test)]
    fn early_len(&self) -> usize {
        self.registry.lock().early.len()
    }
}

/// How a forwarded request ended.
#[derive(Debug)]
pub(crate) enum ForwardOutcome {
    /// The client answered (result or error).
    Answered(Box<JsonRpcResponse>),
    /// No stream to the client, or the client never declared the capability.
    Unreachable,
    /// The stream that carried the request went away before an answer.
    Disconnected,
    TimedOut,
    /// The upstream cancelled the request.
    Cancelled,
}

/// Everything a forward needs from the gateway.
pub(crate) struct ForwardContext<'a> {
    pub sender: Option<&'a ClientRequestSender>,
    pub forwarded: &'a ForwardedRequests,
    pub notifications: Option<&'a NotificationBroadcast>,
    pub session_key: &'a str,
    pub server_id: &'a str,
    pub timeout: Duration,
}

/// Whether the session's client declared `capability` (`sampling`,
/// `elicitation`) in `initialize`.
pub(crate) async fn client_declares(
    session: Option<Arc<RwLock<GatewaySession>>>,
    capability: &str,
) -> bool {
    let Some(session) = session else {
        return false;
    };
    let state = session.read().await;
    state
        .client_capabilities_raw
        .as_ref()
        .and_then(|caps| caps.get(capability))
        .is_some_and(|v| !v.is_null())
}

/// Ask the user, with the sampling approval popup, whether `server_id` may
/// have this sampling request answered by LocalRouter's LLM routing.
pub(crate) async fn approve_sampling(
    manager: &super::sampling_approval::SamplingApprovalManager,
    server_id: &str,
    params: Option<&Value>,
) -> bool {
    let request = params
        .and_then(|p| serde_json::from_value::<crate::protocol::SamplingRequest>(p.clone()).ok())
        .unwrap_or_else(|| crate::protocol::SamplingRequest {
            messages: Vec::new(),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            stop_sequences: None,
            metadata: None,
            model_preferences: None,
            tools: None,
            tool_choice: None,
        });
    matches!(
        manager
            .request_approval(
                uuid::Uuid::new_v4().to_string(),
                server_id.to_string(),
                request,
                None,
            )
            .await,
        Ok(super::sampling_approval::SamplingApprovalAction::Allow)
    )
}

pub(crate) fn sampling_denied(request_id: Value) -> JsonRpcResponse {
    JsonRpcResponse::error(
        request_id,
        crate::protocol::JsonRpcError::custom(
            -32601,
            "Sampling request was denied by user".to_string(),
            None,
        ),
    )
}

/// The upstream's answer when a sampling passthrough produced none.
pub(crate) fn passthrough_failure(request_id: Value, outcome: &ForwardOutcome) -> JsonRpcResponse {
    let (code, message) = match outcome {
        ForwardOutcome::Unreachable => (
            -32601,
            "Sampling is unavailable: the connected MCP client does not support sampling or has no open stream",
        ),
        ForwardOutcome::Disconnected => (
            -32603,
            "Sampling passthrough failed: the MCP client disconnected before answering",
        ),
        ForwardOutcome::TimedOut => (
            -32603,
            "Sampling passthrough timed out waiting for client response",
        ),
        ForwardOutcome::Cancelled => (-32800, "Request cancelled"),
        ForwardOutcome::Answered(_) => (-32603, "Unexpected sampling passthrough state"),
    };
    JsonRpcResponse::error(
        request_id,
        crate::protocol::JsonRpcError::custom(code, message.to_string(), None),
    )
}

/// Forward `method`/`params` (the upstream's request `upstream_id`) to the
/// session's client and wait for its answer, the upstream's cancellation or
/// the timeout.
pub(crate) async fn forward_to_client(
    ctx: ForwardContext<'_>,
    upstream_id: &Value,
    method: &str,
    params: Option<Value>,
) -> ForwardOutcome {
    let Some(sender) = ctx.sender else {
        return ForwardOutcome::Unreachable;
    };
    let forwarded_id = json!(uuid::Uuid::new_v4().to_string());
    let (_guard, cancelled) = ctx
        .forwarded
        .register(ctx.session_key, ctx.server_id, upstream_id);
    if cancelled.is_cancelled() {
        // The upstream cancelled it before it got here: never shown
        return ForwardOutcome::Cancelled;
    }
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(forwarded_id.clone()),
        method: method.to_string(),
        params,
    };
    let Some(answer) = sender(ctx.session_key, request) else {
        return ForwardOutcome::Unreachable;
    };
    tracing::info!(
        "Passthrough: {} from server {} forwarded to the client of session {}",
        method,
        ctx.server_id,
        ctx.session_key
    );
    let (outcome, reason) = tokio::select! {
        biased;
        _ = cancelled.cancelled() => (ForwardOutcome::Cancelled, "Cancelled by the MCP server"),
        answer = tokio::time::timeout(ctx.timeout, answer) => match answer {
            Ok(Ok(response)) => return ForwardOutcome::Answered(Box::new(response)),
            Ok(Err(_)) => return ForwardOutcome::Disconnected,
            Err(_) => (ForwardOutcome::TimedOut, "LocalRouter stopped waiting for an answer"),
        },
    };
    if let Some(broadcast) = ctx.notifications {
        let _ = broadcast.send((
            super::types::session_notification_key(ctx.session_key),
            JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/cancelled".to_string(),
                params: Some(json!({"requestId": forwarded_id, "reason": reason})),
            },
        ));
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    type Captured =
        Arc<parking_lot::Mutex<Vec<(String, JsonRpcRequest, oneshot::Sender<JsonRpcResponse>)>>>;

    fn capturing_sender() -> (ClientRequestSender, Captured) {
        let captured: Captured = Arc::default();
        let sink = captured.clone();
        let sender: ClientRequestSender = Arc::new(move |session: &str, request| {
            let (tx, rx) = oneshot::channel();
            sink.lock().push((session.to_string(), request, tx));
            Some(rx)
        });
        (sender, captured)
    }

    fn ctx<'a>(
        sender: &'a ClientRequestSender,
        forwarded: &'a ForwardedRequests,
        notifications: &'a NotificationBroadcast,
        timeout: Duration,
    ) -> ForwardContext<'a> {
        ForwardContext {
            sender: Some(sender),
            forwarded,
            notifications: Some(notifications),
            session_key: "s1",
            server_id: "srv",
            timeout,
        }
    }

    async fn wait_for_request(captured: &Captured) {
        for _ in 0..200 {
            if !captured.lock().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("request never forwarded");
    }

    #[tokio::test]
    async fn answer_is_returned_under_a_fresh_id_and_the_entry_goes() {
        let (sender, captured) = capturing_sender();
        let forwarded = ForwardedRequests::default();
        let (broadcast, _) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let task = {
            let (sender, forwarded, broadcast) =
                (sender.clone(), forwarded.clone(), broadcast.clone());
            tokio::spawn(async move {
                forward_to_client(
                    ctx(&sender, &forwarded, &broadcast, Duration::from_secs(5)),
                    &json!(7),
                    "sampling/createMessage",
                    Some(json!({"messages": []})),
                )
                .await
            })
        };
        wait_for_request(&captured).await;
        assert_eq!(forwarded.len(), 1);
        let (session, request, tx) = captured.lock().pop().unwrap();
        assert_eq!(session, "s1");
        assert_eq!(request.method, "sampling/createMessage");
        assert_ne!(request.id, Some(json!(7)), "the client sees its own id");
        tx.send(JsonRpcResponse::success(
            request.id.unwrap(),
            json!({"role": "assistant"}),
        ))
        .unwrap();
        match task.await.unwrap() {
            ForwardOutcome::Answered(r) => assert_eq!(r.result.unwrap()["role"], "assistant"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(forwarded.len(), 0);
    }

    #[tokio::test]
    async fn upstream_cancellation_ends_the_wait_and_tells_the_client() {
        let (sender, captured) = capturing_sender();
        let forwarded = ForwardedRequests::default();
        let (broadcast, mut rx) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let task = {
            let (sender, forwarded, broadcast) =
                (sender.clone(), forwarded.clone(), broadcast.clone());
            tokio::spawn(async move {
                forward_to_client(
                    ctx(&sender, &forwarded, &broadcast, Duration::from_secs(30)),
                    &json!("up-1"),
                    "sampling/createMessage",
                    None,
                )
                .await
            })
        };
        wait_for_request(&captured).await;
        let forwarded_id = captured.lock()[0].1.id.clone().unwrap();
        let cancel = |id: Value, session: &str, server: &str| {
            forwarded.cancel_from_upstream(
                session,
                server,
                &JsonRpcNotification {
                    jsonrpc: "2.0".into(),
                    method: "notifications/cancelled".into(),
                    params: Some(json!({"requestId": id})),
                },
            )
        };
        // Another id, another session or another server: not ours
        assert!(!cancel(json!("up-2"), "s1", "srv"));
        assert!(!cancel(json!("up-1"), "s2", "srv"));
        assert!(!cancel(json!("up-1"), "s1", "other"));
        assert!(cancel(json!("up-1"), "s1", "srv"));
        assert!(matches!(task.await.unwrap(), ForwardOutcome::Cancelled));
        let (key, notification) = rx.recv().await.unwrap();
        assert_eq!(key, super::super::types::session_notification_key("s1"));
        assert_eq!(notification.method, "notifications/cancelled");
        assert_eq!(notification.params.unwrap()["requestId"], forwarded_id);
        assert_eq!(forwarded.len(), 0);
    }

    /// The cancellation can overtake its request (two tasks): the request is
    /// then never shown to the client.
    #[tokio::test]
    async fn a_cancellation_that_overtakes_its_request_still_applies() {
        let (sender, captured) = capturing_sender();
        let forwarded = ForwardedRequests::default();
        let (broadcast, _) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let cancel = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/cancelled".into(),
            params: Some(json!({"requestId": 9})),
        };
        assert!(!forwarded.cancel_from_upstream("s1", "srv", &cancel));
        assert_eq!(forwarded.early_len(), 1);
        let outcome = forward_to_client(
            ctx(&sender, &forwarded, &broadcast, Duration::from_secs(5)),
            &json!(9),
            "sampling/createMessage",
            None,
        )
        .await;
        assert!(matches!(outcome, ForwardOutcome::Cancelled));
        assert!(captured.lock().is_empty(), "never sent to the client");
        assert_eq!(forwarded.early_len(), 0);
        assert_eq!(forwarded.len(), 0);
        // Early cancellations are bounded
        for i in 0..(EARLY_CANCEL_CAP + 10) {
            let cancel = JsonRpcNotification {
                params: Some(json!({"requestId": i})),
                ..cancel.clone()
            };
            forwarded.cancel_from_upstream("s1", "srv", &cancel);
        }
        assert_eq!(forwarded.early_len(), EARLY_CANCEL_CAP);
    }

    #[tokio::test]
    async fn timeout_cancels_at_the_client() {
        let (sender, captured) = capturing_sender();
        let forwarded = ForwardedRequests::default();
        let (broadcast, mut rx) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let outcome = forward_to_client(
            ctx(&sender, &forwarded, &broadcast, Duration::from_millis(30)),
            &json!(1),
            "elicitation/create",
            None,
        )
        .await;
        assert!(matches!(outcome, ForwardOutcome::TimedOut));
        let forwarded_id = captured.lock()[0].1.id.clone().unwrap();
        let (_, notification) = rx.recv().await.unwrap();
        assert_eq!(notification.params.unwrap()["requestId"], forwarded_id);
        assert_eq!(forwarded.len(), 0);
    }

    #[tokio::test]
    async fn no_stream_is_unreachable_and_a_dropped_one_disconnected() {
        let forwarded = ForwardedRequests::default();
        let (broadcast, _) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let none: ClientRequestSender = Arc::new(|_, _| None);
        let outcome = forward_to_client(
            ctx(&none, &forwarded, &broadcast, Duration::from_secs(5)),
            &json!(1),
            "sampling/createMessage",
            None,
        )
        .await;
        assert!(matches!(outcome, ForwardOutcome::Unreachable));
        let dropped: ClientRequestSender = Arc::new(|_, _| {
            let (_tx, rx) = oneshot::channel();
            Some(rx)
        });
        let outcome = forward_to_client(
            ctx(&dropped, &forwarded, &broadcast, Duration::from_secs(5)),
            &json!(1),
            "sampling/createMessage",
            None,
        )
        .await;
        assert!(matches!(outcome, ForwardOutcome::Disconnected));
        assert_eq!(forwarded.len(), 0);
    }

    #[tokio::test]
    async fn ending_the_session_ends_its_waits_at_once() {
        let (sender, captured) = capturing_sender();
        let forwarded = ForwardedRequests::default();
        let (broadcast, _rx) = tokio::sync::broadcast::channel(8);
        let broadcast = Arc::new(broadcast);
        let task = {
            let (sender, forwarded, broadcast) =
                (sender.clone(), forwarded.clone(), broadcast.clone());
            tokio::spawn(async move {
                forward_to_client(
                    ctx(&sender, &forwarded, &broadcast, Duration::from_secs(120)),
                    &json!(1),
                    "sampling/createMessage",
                    None,
                )
                .await
            })
        };
        wait_for_request(&captured).await;
        forwarded.cancel_session("another session");
        assert_eq!(forwarded.len(), 1, "other sessions are left alone");
        forwarded.cancel_session("s1");
        let outcome = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the wait ends with its session")
            .unwrap();
        assert!(matches!(outcome, ForwardOutcome::Cancelled), "{outcome:?}");
        assert_eq!(forwarded.len(), 0);
    }

    #[test]
    fn a_finished_wait_leaves_a_newer_one_under_the_same_id() {
        let forwarded = ForwardedRequests::default();
        let (old_guard, _) = forwarded.register("s1", "srv", &json!(1));
        // The upstream reconnected and reuses id 1
        let (_new_guard, new_token) = forwarded.register("s1", "srv", &json!(1));
        drop(old_guard);
        assert_eq!(forwarded.len(), 1, "the newer wait keeps its entry");
        let cancel = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/cancelled".to_string(),
            params: Some(json!({"requestId": 1})),
        };
        assert!(forwarded.cancel_from_upstream("s1", "srv", &cancel));
        assert!(new_token.is_cancelled());
    }
}
