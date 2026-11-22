//! Claude Channels permission relay: the prompts a STDIO bridge session has
//! open, and the wire forms of a prompt and its verdict.
//!
//! Wire form (code.claude.com/docs/en/channels-reference, "Relay permission
//! prompts"): a channel server opts in with
//! `capabilities.experimental["claude/channel/permission"] = {}`. When a
//! tool-approval dialog opens, Claude Code sends
//! `notifications/claude/channel/permission_request` `{request_id, tool_name,
//! description, input_preview}` to it, and the server may answer with
//! `notifications/claude/channel/permission` `{request_id, behavior}`
//! (`allow` or `deny`). Claude Code applies a verdict only for an id it
//! issued and that is still open; the local dialog stays live and the first
//! answer wins.
//!
//! LocalRouter is the channel server toward the bridge session and the
//! client of the upstream channel servers. This table is what lets it accept
//! a verdict only from a server the prompt was sent to, only once, and only
//! for the session that asked.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Synthetic event type of a permission verdict (`<server>__channel.permission`).
/// Its Allow / Ask / Off permission governs the whole relay for that server.
pub const PERMISSION_EVENT_NAME: &str = "channel.permission";
/// Name Monitor records of a forwarded prompt carry
/// (`<server>__channel.permission_request`). Not an event type of its own.
pub const PERMISSION_REQUEST_EVENT_NAME: &str = "channel.permission_request";

/// Longest request id accepted. Claude Code documents five lowercase letters
/// without `l`; the bound is loose so a longer id still relays.
const MAX_REQUEST_ID_LEN: usize = 64;

/// Whether `id` can be a Claude Code permission request id: a short token
/// of ASCII letters and digits.
pub fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_REQUEST_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A permission prompt as Claude Code relays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub input_preview: String,
}

impl PermissionRequest {
    /// Parse the params of `notifications/claude/channel/permission_request`.
    pub fn parse(params: Option<&Value>) -> Result<Self, String> {
        let params = params
            .and_then(Value::as_object)
            .ok_or("params are required")?;
        let field = |name: &str| -> Result<String, String> {
            params
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("{name} must be a string"))
        };
        let request_id = field("request_id")?;
        if !valid_request_id(&request_id) {
            return Err("request_id is not a valid id".into());
        }
        Ok(Self {
            request_id,
            tool_name: field("tool_name")?,
            description: field("description")?,
            input_preview: field("input_preview")?,
        })
    }

    /// The params forwarded upstream: the four documented fields, nothing else.
    pub fn params(&self) -> Value {
        json!({
            "request_id": self.request_id,
            "tool_name": self.tool_name,
            "description": self.description,
            "input_preview": self.input_preview,
        })
    }
}

/// What a verdict decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    Allow,
    Deny,
}

impl Behavior {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// A verdict as an upstream channel server sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub request_id: String,
    pub behavior: Behavior,
}

impl Verdict {
    /// Parse the params of `notifications/claude/channel/permission`.
    pub fn parse(params: Option<&Value>) -> Result<Self, String> {
        let params = params
            .and_then(Value::as_object)
            .ok_or("params are required")?;
        let request_id = params
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|id| valid_request_id(id))
            .ok_or("request_id is not a valid id")?
            .to_string();
        let behavior = params
            .get("behavior")
            .and_then(Value::as_str)
            .and_then(Behavior::parse)
            .ok_or("behavior must be \"allow\" or \"deny\"")?;
        Ok(Self {
            request_id,
            behavior,
        })
    }

    /// The params delivered to Claude Code.
    pub fn params(request_id: &str, behavior: Behavior) -> Value {
        json!({ "request_id": request_id, "behavior": behavior.as_str() })
    }
}

/// Why a verdict was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictRefusal {
    /// The session has no open prompt with that id.
    Unknown,
    /// The prompt was not sent to the server that answered.
    NotATarget,
    /// A verdict for the prompt was already accepted.
    AlreadyAnswered,
    /// The session used the id for more than one prompt, so no verdict can
    /// be matched to one of them (Claude Code drops such verdicts too).
    Repeated,
}

impl VerdictRefusal {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Unknown => "No open permission prompt has this request id",
            Self::NotATarget => "The permission prompt was not sent to this server",
            Self::AlreadyAnswered => "The permission prompt was already answered",
            Self::Repeated => "The request id was used for more than one permission prompt",
        }
    }
}

struct Pending {
    request: PermissionRequest,
    targets: HashSet<String>,
    opened: Instant,
    answered: bool,
    repeated: bool,
}

/// Open permission prompts per downstream session, oldest first.
#[derive(Default)]
pub struct PermissionRequests {
    sessions: HashMap<String, VecDeque<Pending>>,
}

impl PermissionRequests {
    /// Record a prompt sent to `targets`. Returns `false` when the session
    /// already used this id: the prompt must not be relayed, and no verdict
    /// for the id is accepted any more. At most `cap` prompts are kept per
    /// session; the oldest is forgotten first.
    pub fn open(
        &mut self,
        session_key: &str,
        request: PermissionRequest,
        targets: HashSet<String>,
        cap: usize,
    ) -> bool {
        let pending = self.sessions.entry(session_key.to_string()).or_default();
        if let Some(existing) = pending
            .iter_mut()
            .find(|p| p.request.request_id == request.request_id)
        {
            existing.repeated = true;
            return false;
        }
        while pending.len() >= cap.max(1) {
            pending.pop_front();
        }
        pending.push_back(Pending {
            request,
            targets,
            opened: Instant::now(),
            answered: false,
            repeated: false,
        });
        true
    }

    /// Accept the first verdict of `server_id` for an open prompt of the
    /// session, returning the prompt it answers.
    pub fn accept(
        &mut self,
        session_key: &str,
        request_id: &str,
        server_id: &str,
    ) -> Result<PermissionRequest, VerdictRefusal> {
        let pending = self
            .sessions
            .get_mut(session_key)
            .and_then(|p| p.iter_mut().find(|p| p.request.request_id == request_id))
            .ok_or(VerdictRefusal::Unknown)?;
        if pending.repeated {
            return Err(VerdictRefusal::Repeated);
        }
        if !pending.targets.contains(server_id) {
            return Err(VerdictRefusal::NotATarget);
        }
        if pending.answered {
            return Err(VerdictRefusal::AlreadyAnswered);
        }
        pending.answered = true;
        Ok(pending.request.clone())
    }

    /// Forget prompts older than `ttl` and those of sessions that are gone.
    pub fn sweep(&mut self, ttl: Duration, live: impl Fn(&str) -> bool) {
        let now = Instant::now();
        self.sessions.retain(|session_key, pending| {
            if !live(session_key) {
                return false;
            }
            pending.retain(|p| now.duration_since(p.opened) <= ttl);
            !pending.is_empty()
        });
    }

    /// Prompts remembered for a session.
    pub fn len_for_session(&self, session_key: &str) -> usize {
        self.sessions.get(session_key).map_or(0, VecDeque::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            tool_name: "Bash".into(),
            description: "List files".into(),
            input_preview: r#"{"command":"ls"}"#.into(),
        }
    }

    fn targets(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn request_parses_the_documented_fields_and_forwards_only_those() {
        let parsed = PermissionRequest::parse(Some(&json!({
            "request_id": "abcde", "tool_name": "Bash", "description": "List files",
            "input_preview": "{\"command\":\"ls\"}", "extra": {"x": 1}
        })))
        .unwrap();
        assert_eq!(parsed, request("abcde"));
        assert_eq!(
            parsed.params(),
            json!({"request_id": "abcde", "tool_name": "Bash", "description": "List files",
                "input_preview": "{\"command\":\"ls\"}"})
        );
    }

    #[test]
    fn request_rejects_missing_fields_and_bad_ids() {
        assert!(PermissionRequest::parse(None).is_err());
        assert!(PermissionRequest::parse(Some(&json!([]))).is_err());
        for bad in [
            json!({"request_id": "abcde", "tool_name": "Bash", "description": "d"}),
            json!({"request_id": 5, "tool_name": "Bash", "description": "d", "input_preview": ""}),
            json!({"request_id": "", "tool_name": "Bash", "description": "d", "input_preview": ""}),
            json!({"request_id": "ab cd", "tool_name": "Bash", "description": "d", "input_preview": ""}),
            json!({"request_id": "a".repeat(65), "tool_name": "Bash", "description": "d", "input_preview": ""}),
            json!({"request_id": "abcde", "tool_name": 1, "description": "d", "input_preview": ""}),
        ] {
            assert!(PermissionRequest::parse(Some(&bad)).is_err(), "{bad}");
        }
        assert!(valid_request_id("abcde"));
        assert!(valid_request_id(&"a".repeat(64)));
    }

    #[test]
    fn verdict_parses_allow_and_deny_only() {
        let allow =
            Verdict::parse(Some(&json!({"request_id": "abcde", "behavior": "allow"}))).unwrap();
        assert_eq!(allow.behavior, Behavior::Allow);
        let deny =
            Verdict::parse(Some(&json!({"request_id": "abcde", "behavior": "deny"}))).unwrap();
        assert_eq!(deny.behavior, Behavior::Deny);
        assert_eq!(
            Verdict::params("abcde", Behavior::Deny),
            json!({"request_id": "abcde", "behavior": "deny"})
        );
        for bad in [
            json!({"request_id": "abcde", "behavior": "always"}),
            json!({"request_id": "abcde"}),
            json!({"behavior": "allow"}),
            json!({"request_id": "../x", "behavior": "allow"}),
        ] {
            assert!(Verdict::parse(Some(&bad)).is_err(), "{bad}");
        }
        assert!(Verdict::parse(None).is_err());
    }

    #[test]
    fn first_verdict_from_a_target_wins() {
        let mut table = PermissionRequests::default();
        assert!(table.open("s1", request("abcde"), targets(&["tg", "dc"]), 8));

        assert_eq!(
            table.accept("s1", "zzzzz", "tg"),
            Err(VerdictRefusal::Unknown)
        );
        // Another session's id is not this session's
        assert_eq!(
            table.accept("s2", "abcde", "tg"),
            Err(VerdictRefusal::Unknown)
        );
        // A server the prompt never went to cannot answer it
        assert_eq!(
            table.accept("s1", "abcde", "other"),
            Err(VerdictRefusal::NotATarget)
        );
        assert_eq!(table.accept("s1", "abcde", "tg"), Ok(request("abcde")));
        assert_eq!(
            table.accept("s1", "abcde", "dc"),
            Err(VerdictRefusal::AlreadyAnswered)
        );
        assert_eq!(
            table.accept("s1", "abcde", "tg"),
            Err(VerdictRefusal::AlreadyAnswered)
        );
    }

    #[test]
    fn a_repeated_id_is_not_relayed_and_takes_no_verdict() {
        let mut table = PermissionRequests::default();
        assert!(table.open("s1", request("abcde"), targets(&["tg"]), 8));
        assert!(!table.open("s1", request("abcde"), targets(&["tg"]), 8));
        assert_eq!(
            table.accept("s1", "abcde", "tg"),
            Err(VerdictRefusal::Repeated)
        );
        // The same id in another session is unrelated
        assert!(table.open("s2", request("abcde"), targets(&["tg"]), 8));
        assert!(table.accept("s2", "abcde", "tg").is_ok());
    }

    #[test]
    fn the_cap_forgets_the_oldest_prompt() {
        let mut table = PermissionRequests::default();
        for id in ["aaaaa", "bbbbb", "ccccc"] {
            assert!(table.open("s1", request(id), targets(&["tg"]), 2));
        }
        assert_eq!(table.len_for_session("s1"), 2);
        assert_eq!(
            table.accept("s1", "aaaaa", "tg"),
            Err(VerdictRefusal::Unknown)
        );
        assert!(table.accept("s1", "bbbbb", "tg").is_ok());
        assert!(table.accept("s1", "ccccc", "tg").is_ok());
    }

    #[test]
    fn sweep_drops_expired_prompts_and_dead_sessions() {
        let mut table = PermissionRequests::default();
        table.open("live", request("aaaaa"), targets(&["tg"]), 8);
        table.open("gone", request("bbbbb"), targets(&["tg"]), 8);
        table.sweep(Duration::from_secs(3600), |s| s == "live");
        assert_eq!(table.len_for_session("live"), 1);
        assert_eq!(table.len_for_session("gone"), 0);

        std::thread::sleep(Duration::from_millis(5));
        table.sweep(Duration::ZERO, |_| true);
        assert_eq!(table.len_for_session("live"), 0);
        assert_eq!(
            table.accept("live", "aaaaa", "tg"),
            Err(VerdictRefusal::Unknown)
        );
    }
}
