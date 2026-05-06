//! Disk discovery + live-process detection for coding-agent sessions.
//!
//! Some coding-agent CLIs persist every session to a well-known
//! directory (Claude Code → `~/.claude/projects/<encoded-cwd>/<id>.jsonl`,
//! ACP-harnessed agents → `~/.vibe-kanban/<agent>_sessions/<id>.jsonl`).
//! [`CodingAgentManager::discover_disk_sessions`](crate::manager::CodingAgentManager::discover_disk_sessions)
//! enumerates those records so callers can present them to a user
//! ("attach to one of these"), then later resume them via
//! [`start_session_with_resume`](crate::manager::CodingAgentManager::start_session_with_resume).
//!
//! Each discovered entry is augmented with a [`Liveness`] tag —
//! cheap heuristic detection of whether a process is currently
//! driving the session, so callers can refuse / warn before resuming
//! a session whose CLI is still alive.
//!
//! Per-agent feasibility is documented inline; agents whose CLI keeps
//! session state server-side (Codex, Opencode, Amp) or whose layout
//! isn't enumerable (Cursor, Droid) return `Ok(vec![])` and rely on
//! the caller-supplied `agent_session_id` for attach.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lr_config::CodingAgentType;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One discovered session, suitable for `--resume <agent_session_id>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSession {
    /// Which coding agent persisted this session.
    pub agent: CodingAgentType,
    /// The agent's own session id — opaque, suitable for resume.
    pub agent_session_id: String,
    /// Working directory the session was last running in.
    pub working_directory: PathBuf,
    /// File mtime, last-message timestamp, or other signal of recency.
    pub last_active_at: SystemTime,
    /// How many recorded messages / events this session has.
    pub message_count: u32,
    /// Short preview of the first user message, trimmed to ~200 chars.
    pub summary: Option<String>,
    /// Liveness signal — is a process currently driving this session?
    pub liveness: Liveness,
}

/// Result of the per-discovery live-process scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Liveness {
    /// A process whose argv references this session id was found.
    LivePid {
        /// PID of the matching process.
        pid: u32,
    },
    /// Process scan ran and matched nothing — believed safe to attach.
    NotFound,
    /// Process scan unavailable on this platform / environment. Treat
    /// as potentially live; callers may attach but should disclose.
    Unknown,
}

impl Liveness {
    /// Convenience: is the session currently being driven?
    pub fn is_live(&self) -> bool {
        matches!(self, Liveness::LivePid { .. })
    }
}

/// Filter on a discovery call.
#[derive(Debug, Clone, Default)]
pub struct DiscoverFilter {
    /// Restrict to these agent kinds. `None` = every supported agent.
    pub agents: Option<Vec<CodingAgentType>>,
    /// Restrict to sessions whose `working_directory` matches this
    /// path. `None` = every cwd.
    pub working_directory: Option<PathBuf>,
    /// Cap on the merged result set (sorted by `last_active_at` desc).
    /// 0 means use the default (`DEFAULT_LIMIT`).
    pub limit: usize,
    /// When set, restrict to a single `agent_session_id` — the targeted
    /// "does this exist?" lookup used by attach validation. Behaves
    /// like a filter on the post-scan list.
    pub session_id: Option<String>,
}

/// Default cap when `DiscoverFilter.limit == 0`.
pub const DEFAULT_LIMIT: usize = 25;

/// Per-agent on-disk discovery contract. One impl per
/// [`CodingAgentType`] variant — including trivially-empty ones, so a
/// new agent kind never silently defaults to "yes, discover everything"
/// or "no, never discover".
pub(crate) trait DiskDiscovery {
    /// Enumerate sessions on disk. Per-file IO errors must `warn!` and
    /// be skipped — never abort the whole scan. A missing root is not
    /// an error; it just yields an empty list.
    fn discover(
        &self,
        working_directory: Option<&Path>,
        session_id: Option<&str>,
    ) -> Vec<DiscoveredSession>;
}

/// Resolve the per-agent discovery impl. Returns the trivial
/// empty-stub impl for agents we can't enumerate in v1.
pub(crate) fn discoverer_for(agent: CodingAgentType) -> Box<dyn DiskDiscovery + Send + Sync> {
    match agent {
        CodingAgentType::ClaudeCode => Box::new(ClaudeCodeDiscovery),
        CodingAgentType::GeminiCli => Box::new(AcpDiscovery {
            agent: CodingAgentType::GeminiCli,
            namespace: "gemini_sessions",
        }),
        CodingAgentType::Copilot => Box::new(AcpDiscovery {
            agent: CodingAgentType::Copilot,
            namespace: "copilot_sessions",
        }),
        CodingAgentType::QwenCode => Box::new(AcpDiscovery {
            agent: CodingAgentType::QwenCode,
            namespace: "qwen_sessions",
        }),
        // Cursor / Droid: CLI-managed state, no documented enumerable
        // path. Resume by id still works; discovery returns empty.
        CodingAgentType::Cursor | CodingAgentType::Droid => Box::new(EmptyDiscovery),
        // Codex / Opencode / Amp: server-side state. Future server-API
        // discovery is possible; out of scope for v1.
        CodingAgentType::Codex | CodingAgentType::Opencode | CodingAgentType::Amp => {
            Box::new(EmptyDiscovery)
        }
        // Aider: not session-resumable.
        CodingAgentType::Aider => Box::new(EmptyDiscovery),
    }
}

// ── Empty stub for agents without enumerable on-disk storage ─────────

struct EmptyDiscovery;

impl DiskDiscovery for EmptyDiscovery {
    fn discover(&self, _: Option<&Path>, _: Option<&str>) -> Vec<DiscoveredSession> {
        Vec::new()
    }
}

// ── Claude Code: native CLI storage at ~/.claude/projects/ ──────────

struct ClaudeCodeDiscovery;

impl DiskDiscovery for ClaudeCodeDiscovery {
    fn discover(
        &self,
        working_directory: Option<&Path>,
        session_id: Option<&str>,
    ) -> Vec<DiscoveredSession> {
        let Some(root) = claude_projects_root() else {
            return Vec::new();
        };
        let read_dir = match std::fs::read_dir(&root) {
            Ok(it) => it,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    warn!(root = %root.display(), error = %err, "claude-code: failed to read projects root");
                }
                return Vec::new();
            }
        };

        // Optional cwd filter — encode the wanted path the way Claude
        // Code does (replace `/` and `.` with `-`, prefix `-`) and only
        // descend into that one subdir.
        let wanted_subdir = working_directory.map(claude_encode_path);

        let mut out = Vec::new();
        for entry in read_dir.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = match dir.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            if let Some(ref wanted) = wanted_subdir {
                if dir_name != *wanted {
                    continue;
                }
            }
            // Best-effort decode — we use the result for the cwd field;
            // mismatches against the on-disk dir name are recoverable.
            let cwd = claude_decode_dirname(&dir_name);
            scan_claude_subdir(&dir, &cwd, session_id, &mut out);
        }
        out
    }
}

fn claude_projects_root() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CLAUDE_HOME") {
        return Some(PathBuf::from(home).join("projects"));
    }
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

/// Mirrors Claude Code's path-encoding: replace `/` and `.` with `-`,
/// prepend `-` so the absolute-path leading slash is preserved.
fn claude_encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len() + 1);
    if !s.starts_with('-') {
        // For an absolute path the first char is already `/` which the
        // replace below turns into `-`, so this is a no-op for typical
        // input; included for paths that come in without a leading slash.
    }
    for ch in s.chars() {
        match ch {
            '/' | '.' => out.push('-'),
            other => out.push(other),
        }
    }
    out
}

/// Best-effort inverse — turn `-Users-matus-dev-direktor` back into
/// `/Users/matus/dev/direktor`. Note: this is lossy because the original
/// encoding collapses `/` and `.` onto the same character; we only
/// surface the result as the cwd "label" and the LLM should still rely
/// on `working_directory` filtering when it cares about an exact match.
fn claude_decode_dirname(name: &str) -> PathBuf {
    PathBuf::from(name.replace('-', "/"))
}

fn scan_claude_subdir(
    dir: &Path,
    cwd: &Path,
    session_id_filter: Option<&str>,
    out: &mut Vec<DiscoveredSession>,
) {
    let read = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(err) => {
            warn!(dir = %dir.display(), error = %err, "claude-code: failed to read project subdir");
            return;
        }
    };
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(want) = session_id_filter {
            if stem != want {
                continue;
            }
        }
        let meta = match path.metadata() {
            Ok(m) => m,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "claude-code: stat failed");
                continue;
            }
        };
        let last_active_at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (message_count, summary) = parse_claude_jsonl(&path);
        out.push(DiscoveredSession {
            agent: CodingAgentType::ClaudeCode,
            agent_session_id: stem.to_owned(),
            working_directory: cwd.to_path_buf(),
            last_active_at,
            message_count,
            summary,
            liveness: Liveness::Unknown,
        });
    }
}

/// Parse the JSONL: count lines (cheap upper bound on message count)
/// and pull the first user message text for `summary`.
fn parse_claude_jsonl(path: &Path) -> (u32, Option<String>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            warn!(path = %path.display(), error = %err, "claude-code: read failed");
            return (0, None);
        }
    };
    let mut count = 0u32;
    let mut summary: Option<String> = None;
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        count = count.saturating_add(1);
        if summary.is_some() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Claude Code's JSONL uses {"type":"user","message":{"content":[{"type":"text","text":"..."}]}}
        if v.get("type").and_then(|x| x.as_str()) == Some("user") {
            let text = v
                .pointer("/message/content/0/text")
                .and_then(|x| x.as_str())
                .or_else(|| v.pointer("/message/content").and_then(|x| x.as_str()))
                .or_else(|| v.get("content").and_then(|x| x.as_str()));
            if let Some(t) = text {
                summary = Some(trim_summary(t));
            }
        }
    }
    (count, summary)
}

fn trim_summary(s: &str) -> String {
    const MAX: usize = 200;
    let one_line = s.replace('\n', " ");
    if one_line.chars().count() <= MAX {
        one_line
    } else {
        let mut buf = String::new();
        for (i, ch) in one_line.chars().enumerate() {
            if i >= MAX {
                break;
            }
            buf.push(ch);
        }
        buf.push('…');
        buf
    }
}

// ── ACP-harness agents (gemini / copilot / qwen-code) ───────────────

struct AcpDiscovery {
    agent: CodingAgentType,
    namespace: &'static str,
}

impl DiskDiscovery for AcpDiscovery {
    fn discover(
        &self,
        working_directory: Option<&Path>,
        session_id: Option<&str>,
    ) -> Vec<DiscoveredSession> {
        let Some(root) = acp_namespace_root(self.namespace) else {
            return Vec::new();
        };
        let read = match std::fs::read_dir(&root) {
            Ok(it) => it,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    warn!(root = %root.display(), error = %err, ns = %self.namespace, "acp: failed to read sessions root");
                }
                return Vec::new();
            }
        };

        // The ACP harness doesn't record cwd in the JSONL header (the
        // upstream `SessionManager` is namespace-agnostic), so a
        // `working_directory` filter is applied as a post-scan no-op
        // here — every entry has `working_directory = root`. The
        // orchestrator-side dedup against currently-managed sessions
        // is the more useful filter.
        let _ = working_directory;
        let mut out = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_owned(),
                None => continue,
            };
            if let Some(want) = session_id {
                if stem != want {
                    continue;
                }
            }
            let meta = match path.metadata() {
                Ok(m) => m,
                Err(err) => {
                    warn!(path = %path.display(), error = %err, "acp: stat failed");
                    continue;
                }
            };
            let last_active_at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let (message_count, summary) = parse_acp_jsonl(&path);
            out.push(DiscoveredSession {
                agent: self.agent,
                agent_session_id: stem,
                working_directory: root.clone(),
                last_active_at,
                message_count,
                summary,
                liveness: Liveness::Unknown,
            });
        }
        out
    }
}

fn acp_namespace_root(namespace: &str) -> Option<PathBuf> {
    let mut vk = dirs::home_dir()?.join(".vibe-kanban");
    if cfg!(debug_assertions) {
        vk = vk.join("dev");
    }
    Some(vk.join(namespace))
}

fn parse_acp_jsonl(path: &Path) -> (u32, Option<String>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            warn!(path = %path.display(), error = %err, "acp: read failed");
            return (0, None);
        }
    };
    let mut count = 0u32;
    let mut summary: Option<String> = None;
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        count = count.saturating_add(1);
        if summary.is_some() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Normalised ACP shape (executors/acp/session.rs::normalize_session_event)
        // — pure-text user messages serialize as `{"user": "<text>"}`.
        if let Some(text) = v.get("user").and_then(|x| x.as_str()) {
            summary = Some(trim_summary(text));
        }
    }
    (count, summary)
}

// ── Live-process detection ──────────────────────────────────────────

/// Scan running processes for any whose argv contains one of the
/// supplied needles. Returns a parallel `Vec<Liveness>` aligned with
/// `needles`. If the process snapshot can't be taken, every entry is
/// `Liveness::Unknown`.
///
/// Heuristic — argv-string match:
/// - For Claude Code, the underlying process runs with `--resume <id>`,
///   so the session id appears as its own argv token.
/// - For ACP-harness agents, the harness launches the underlying CLI
///   with the session-file path on the command line, so `<id>.jsonl`
///   appears as a substring of one of the argv elements.
///
/// Both cases are covered by a single substring match against each
/// needle string.
pub(crate) fn detect_live_pids(needles: &[&str]) -> Vec<Liveness> {
    if needles.is_empty() {
        return Vec::new();
    }

    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

    let mut sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::new()),
    );
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut out: Vec<Liveness> = vec![Liveness::NotFound; needles.len()];
    for (pid, proc_) in sys.processes() {
        for (i, needle) in needles.iter().enumerate() {
            if matches!(out[i], Liveness::LivePid { .. }) {
                continue;
            }
            if proc_
                .cmd()
                .iter()
                .any(|arg| arg.to_string_lossy().contains(needle))
            {
                out[i] = Liveness::LivePid {
                    pid: pid.as_u32(),
                };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_serialises_with_kind_tag() {
        let live = Liveness::LivePid { pid: 42 };
        let json = serde_json::to_value(&live).unwrap();
        assert_eq!(json["kind"], "live_pid");
        assert_eq!(json["pid"], 42);

        let nf = Liveness::NotFound;
        let json = serde_json::to_value(&nf).unwrap();
        assert_eq!(json["kind"], "not_found");
    }

    #[test]
    fn claude_encode_round_trips_simple_path() {
        let p = Path::new("/Users/matus/dev/direktor");
        let encoded = claude_encode_path(p);
        assert_eq!(encoded, "-Users-matus-dev-direktor");
    }

    #[test]
    fn discoverer_returns_empty_for_unsupported_agents() {
        for agent in [
            CodingAgentType::Codex,
            CodingAgentType::Opencode,
            CodingAgentType::Amp,
            CodingAgentType::Cursor,
            CodingAgentType::Droid,
            CodingAgentType::Aider,
        ] {
            let entries = discoverer_for(agent).discover(None, None);
            assert!(
                entries.is_empty(),
                "{:?} should return empty discovery in v1",
                agent
            );
        }
    }

    #[test]
    fn detect_live_pids_returns_not_found_for_random_uuid() {
        let result = detect_live_pids(&["00000000-0000-0000-0000-000000000000"]);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Liveness::NotFound));
    }
}
