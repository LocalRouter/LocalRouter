//! Per-event Ask: occurrences held before they enter a client's journal.
//!
//! Held events queue per (client, event type) and are decided in order, so a
//! client never sees event N+1 of a type before an approved event N; other
//! types are not blocked. A held event waits for its decision indefinitely;
//! each client holds at most `ask_hold_cap` events (an internal bound, see
//! `limits.rs`).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::journal::Occurrence;
use crate::gateway::firewall::FirewallApprovalAction;

/// A decision about a held event (or a webhook URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushDecisionAction {
    /// Deliver this event (with `edited_data` when given).
    Allow,
    /// Drop this event for this client.
    Deny,
    /// Allow this event type for the rest of this LocalRouter run.
    AllowSession,
    /// Turn this event type off for the rest of this LocalRouter run.
    DenySession,
    /// Allow this event type permanently (written to the client's config).
    AllowAlways,
    /// Turn this event type off permanently (written to the client's config).
    DenyAlways,
}

impl PushDecisionAction {
    pub fn allows(&self) -> bool {
        matches!(self, Self::Allow | Self::AllowSession | Self::AllowAlways)
    }

    /// Applies to every later event of the type, not just this one.
    pub fn is_type_wide(&self) -> bool {
        !matches!(self, Self::Allow | Self::Deny)
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::AllowAlways | Self::DenyAlways)
    }

    /// The permission a type-wide decision sets.
    pub fn permission(&self) -> Option<lr_config::PushPermission> {
        match self {
            Self::AllowSession | Self::AllowAlways => Some(lr_config::PushPermission::Allow),
            Self::DenySession | Self::DenyAlways => Some(lr_config::PushPermission::Off),
            Self::Allow | Self::Deny => None,
        }
    }
}

impl From<&FirewallApprovalAction> for PushDecisionAction {
    fn from(action: &FirewallApprovalAction) -> Self {
        match action {
            FirewallApprovalAction::AllowOnce
            | FirewallApprovalAction::RedactOnce
            | FirewallApprovalAction::Allow1Minute
            | FirewallApprovalAction::Allow1Hour
            | FirewallApprovalAction::AllowCategories => Self::Allow,
            FirewallApprovalAction::AllowSession => Self::AllowSession,
            FirewallApprovalAction::AllowPermanent => Self::AllowAlways,
            FirewallApprovalAction::DenySession => Self::DenySession,
            FirewallApprovalAction::DenyAlways => Self::DenyAlways,
            FirewallApprovalAction::Deny
            | FirewallApprovalAction::Deny1Hour
            | FirewallApprovalAction::BlockCategories
            | FirewallApprovalAction::DisableClient => Self::Deny,
        }
    }
}

impl PushDecisionAction {
    /// The firewall action that delivers this decision to a waiting popup.
    pub fn firewall_action(&self) -> FirewallApprovalAction {
        match self {
            Self::Allow => FirewallApprovalAction::AllowOnce,
            Self::Deny => FirewallApprovalAction::Deny,
            Self::AllowSession => FirewallApprovalAction::AllowSession,
            Self::DenySession => FirewallApprovalAction::DenySession,
            Self::AllowAlways => FirewallApprovalAction::AllowPermanent,
            Self::DenyAlways => FirewallApprovalAction::DenyAlways,
        }
    }
}

/// A decision with an optional edit of the event's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushDecision {
    pub action: PushDecisionAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_data: Option<Value>,
}

impl PushDecision {
    pub fn new(action: PushDecisionAction) -> Self {
        Self {
            action,
            edited_data: None,
        }
    }
}

/// One held event.
#[derive(Debug, Clone)]
pub struct HeldEvent {
    pub hold_id: String,
    pub client_id: String,
    pub client_name: String,
    pub occurrence: Arc<Occurrence>,
    pub held_at: Instant,
    pub held_at_utc: chrono::DateTime<chrono::Utc>,
    pub monitor_id: String,
    /// Decided while queued behind another event; applied in order.
    pub decision: Option<PushDecision>,
}

/// A held event as the UI lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldPushEventInfo {
    pub hold_id: String,
    pub client_id: String,
    pub client_name: String,
    pub server_id: String,
    pub server_name: String,
    pub event_name: String,
    pub qualified_name: String,
    pub event_id: String,
    pub ingress_mode: String,
    pub data: Value,
    pub held_at: chrono::DateTime<chrono::Utc>,
    pub held_secs: u64,
    /// Position in its (client, event type) queue; 0 is being asked now.
    pub position: usize,
    /// Firewall request id of the popup asking about it, while one is open.
    pub firewall_request_id: Option<String>,
    /// Already decided, waiting for the events ahead of it.
    pub decided: bool,
}

/// Why an event was not held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldCapReached;

/// The queues of held events.
#[derive(Default)]
pub struct AskQueues {
    queues: HashMap<(String, String), VecDeque<HeldEvent>>,
    per_client: HashMap<String, usize>,
}

impl AskQueues {
    /// Hold an event. Returns whether its queue was empty (the caller starts
    /// a worker for it), or [`HoldCapReached`].
    pub fn push(&mut self, event: HeldEvent, cap: usize) -> Result<bool, HoldCapReached> {
        let count = self.per_client.entry(event.client_id.clone()).or_default();
        if *count >= cap {
            return Err(HoldCapReached);
        }
        *count += 1;
        let queue = self
            .queues
            .entry((event.client_id.clone(), event.occurrence.name.clone()))
            .or_default();
        let was_empty = queue.is_empty();
        queue.push_back(event);
        Ok(was_empty)
    }

    /// The head of a queue.
    pub fn front(&self, client_id: &str, name: &str) -> Option<HeldEvent> {
        self.queues
            .get(&(client_id.to_string(), name.to_string()))
            .and_then(|q| q.front().cloned())
    }

    /// Remove the head if it is `hold_id`.
    pub fn pop(&mut self, client_id: &str, name: &str, hold_id: &str) -> Option<HeldEvent> {
        let key = (client_id.to_string(), name.to_string());
        let queue = self.queues.get_mut(&key)?;
        if queue.front().map(|e| e.hold_id.as_str()) != Some(hold_id) {
            return None;
        }
        let event = queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(&key);
        }
        if let Some(count) = self.per_client.get_mut(client_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.per_client.remove(client_id);
            }
        }
        event
    }

    /// Record a decision on a queued event. Returns its (client, name).
    pub fn decide(&mut self, hold_id: &str, decision: PushDecision) -> Option<(String, String)> {
        for ((client, name), queue) in self.queues.iter_mut() {
            if let Some(event) = queue.iter_mut().find(|e| e.hold_id == hold_id) {
                event.decision = Some(decision);
                return Some((client.clone(), name.clone()));
            }
        }
        None
    }

    /// Record a decision on every undecided event of one queue.
    pub fn decide_all(&mut self, client_id: &str, name: &str, decision: &PushDecision) -> usize {
        let Some(queue) = self
            .queues
            .get_mut(&(client_id.to_string(), name.to_string()))
        else {
            return 0;
        };
        let mut count = 0;
        for event in queue.iter_mut().filter(|e| e.decision.is_none()) {
            event.decision = Some(decision.clone());
            count += 1;
        }
        count
    }

    pub fn find(&self, hold_id: &str) -> Option<HeldEvent> {
        self.queues
            .values()
            .flat_map(|q| q.iter())
            .find(|e| e.hold_id == hold_id)
            .cloned()
    }

    pub fn count_for_client(&self, client_id: &str) -> usize {
        self.per_client.get(client_id).copied().unwrap_or(0)
    }

    pub fn count_for_type(&self, client_id: &str, name: &str) -> usize {
        self.queues
            .get(&(client_id.to_string(), name.to_string()))
            .map_or(0, |q| q.len())
    }

    /// Every held event with its queue position.
    pub fn list(&self) -> Vec<(usize, HeldEvent)> {
        self.queues
            .values()
            .flat_map(|q| q.iter().cloned().enumerate())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::push_relay::journal::IngressMode;

    fn held(client: &str, name: &str, id: &str) -> HeldEvent {
        HeldEvent {
            hold_id: id.to_string(),
            client_id: client.to_string(),
            client_name: client.to_string(),
            occurrence: Arc::new(Occurrence {
                event_id: format!("e-{id}"),
                name: name.to_string(),
                server_id: "s".into(),
                server_slug: "s".into(),
                upstream_name: name.into(),
                timestamp: chrono::Utc::now(),
                data: Value::Null,
                upstream_cursor: None,
                ingress: IngressMode::Push,
                args_key: "{}".into(),
                origin_session: None,
            }),
            held_at: Instant::now(),
            held_at_utc: chrono::Utc::now(),
            monitor_id: String::new(),
            decision: None,
        }
    }

    #[test]
    fn queues_are_fifo_per_client_and_type() {
        let mut queues = AskQueues::default();
        assert_eq!(queues.push(held("c", "a", "1"), 10), Ok(true));
        assert_eq!(queues.push(held("c", "a", "2"), 10), Ok(false));
        assert_eq!(queues.push(held("c", "b", "3"), 10), Ok(true));
        assert_eq!(queues.push(held("d", "a", "4"), 10), Ok(true));
        assert_eq!(queues.front("c", "a").unwrap().hold_id, "1");
        // Only the head can be popped
        assert!(queues.pop("c", "a", "2").is_none());
        assert_eq!(queues.pop("c", "a", "1").unwrap().hold_id, "1");
        assert_eq!(queues.front("c", "a").unwrap().hold_id, "2");
        assert_eq!(queues.count_for_client("c"), 2);
        assert_eq!(queues.count_for_type("c", "a"), 1);
    }

    #[test]
    fn hold_cap_is_per_client() {
        let mut queues = AskQueues::default();
        assert!(queues.push(held("c", "a", "1"), 2).is_ok());
        assert!(queues.push(held("c", "b", "2"), 2).is_ok());
        assert_eq!(queues.push(held("c", "a", "3"), 2), Err(HoldCapReached));
        assert!(queues.push(held("d", "a", "4"), 2).is_ok());
        queues.pop("c", "a", "1");
        assert!(queues.push(held("c", "a", "5"), 2).is_ok());
    }

    #[test]
    fn decisions_attach_to_queued_events() {
        let mut queues = AskQueues::default();
        queues.push(held("c", "a", "1"), 10).unwrap();
        queues.push(held("c", "a", "2"), 10).unwrap();
        queues.push(held("c", "a", "3"), 10).unwrap();
        assert_eq!(
            queues.decide("2", PushDecision::new(PushDecisionAction::Deny)),
            Some(("c".into(), "a".into()))
        );
        assert_eq!(
            queues.decide_all("c", "a", &PushDecision::new(PushDecisionAction::Allow)),
            2
        );
        assert_eq!(
            queues.find("2").unwrap().decision.unwrap().action,
            PushDecisionAction::Deny
        );
        assert!(queues
            .decide("missing", PushDecision::new(PushDecisionAction::Deny))
            .is_none());
    }

    #[test]
    fn firewall_actions_map_both_ways() {
        for action in [
            PushDecisionAction::Allow,
            PushDecisionAction::Deny,
            PushDecisionAction::AllowSession,
            PushDecisionAction::DenySession,
            PushDecisionAction::AllowAlways,
            PushDecisionAction::DenyAlways,
        ] {
            assert_eq!(PushDecisionAction::from(&action.firewall_action()), action);
        }
        assert_eq!(
            PushDecisionAction::from(&FirewallApprovalAction::Allow1Hour),
            PushDecisionAction::Allow
        );
    }
}
