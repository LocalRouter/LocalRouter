//! SQLite-backed session store for the Responses API.
//!
//! The OpenAI Responses API lets clients continue a conversation by
//! passing `previous_response_id` instead of re-sending the full
//! history. To serve `/v1/responses` on top of chat-completions
//! providers we record each stored response locally and rehydrate its
//! conversation on follow-up turns.
//!
//! Retention policy (matches the approved plan):
//!
//! - `store: true` on the request → record this turn's full message
//!   history + the final cached response; retention defaults to 30
//!   days (OpenAI's documented window).
//! - `store: false` → nothing is persisted; the streaming reply is
//!   piped through and the row never exists.
//! - An `active_window_hours` cap (default 24h) bounds how long a
//!   `previous_response_id` chain can sit idle before we treat it as
//!   cold. Both TTLs are configurable via `lr-config` fields.
//!
//! Design modelled on `lr-monitoring::storage` (rusqlite, WAL mode,
//! `Arc<Mutex<Connection>>`, `INSERT OR REPLACE`).

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use lr_providers::{ChatMessage, Tool};

/// Row stored in the `responses_sessions` table. The `final_response_json`
/// is cached so idempotent re-reads of the same response id return the
/// same body without hitting the upstream again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesSession {
    /// UUID we mint for each stored response. Clients pass this back
    /// as `previous_response_id`.
    pub id: String,
    /// Logical owner (LocalRouter client id / API-key id).
    pub client_id: String,
    /// The id we were continuing from, if any.
    pub previous_response_id: Option<String>,
    pub model: String,
    pub created_at: i64,
    pub last_activity: i64,
    pub store: bool,
    pub metadata_json: Option<String>,
    /// Cumulative chat-completion history that produced this response,
    /// including the assistant's final turn. Used to rebuild the
    /// request when `previous_response_id` is referenced.
    pub messages_json: String,
    /// Tools carried across the chain so follow-up turns don't have
    /// to re-specify them.
    pub tools_json: Option<String>,
    /// Full cached response body (JSON) for idempotent re-reads.
    pub final_response_json: Option<String>,
}

/// Retention configuration. Loaded from `lr-config` in production.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub retention_days: i64,
    pub active_window_hours: i64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            retention_days: 30,
            active_window_hours: 24,
        }
    }
}

/// Thread-safe session store. Cheap to clone (wraps an `Arc<Mutex<Connection>>`).
#[derive(Clone)]
pub struct ResponsesSessionStore {
    conn: Arc<Mutex<Connection>>,
}

impl ResponsesSessionStore {
    /// Open (or create) the SQLite DB at `path`, applying WAL mode and
    /// initializing the schema.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            // Best-effort — if mkdir fails, `Connection::open` will
            // return a concrete error below.
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        // journal_mode returns a result row, so use prepare+query
        let _ = conn
            .prepare("PRAGMA journal_mode=WAL")?
            .query_row([], |_| Ok(()));
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        Self::init_schema(&conn)?;
        info!("Responses session store opened at {}", path.display());
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// In-memory variant used in tests and when the caller doesn't
    /// want disk persistence.
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS responses_sessions (
                id TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                previous_response_id TEXT,
                model TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity INTEGER NOT NULL,
                store INTEGER NOT NULL,
                metadata_json TEXT,
                messages_json TEXT NOT NULL,
                tools_json TEXT,
                final_response_json TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_client_id
                ON responses_sessions (client_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_last_activity
                ON responses_sessions (last_activity);",
        )
    }

    /// Persist a new turn. Overwrites an existing row if the same id
    /// is saved twice (caller is responsible for id uniqueness).
    pub fn insert(&self, session: &ResponsesSession) -> rusqlite::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO responses_sessions
              (id, client_id, previous_response_id, model, created_at,
               last_activity, store, metadata_json, messages_json,
               tools_json, final_response_json)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session.id,
                session.client_id,
                session.previous_response_id,
                session.model,
                session.created_at,
                session.last_activity,
                session.store as i64,
                session.metadata_json,
                session.messages_json,
                session.tools_json,
                session.final_response_json,
            ],
        )?;
        Ok(())
    }

    /// Fetch a session by id. Enforces the `active_window_hours`
    /// check: if the chain has been idle longer than that, the row is
    /// treated as expired and `None` returned (the row remains in the
    /// DB until the retention sweeper drops it).
    pub fn get_active(
        &self,
        id: &str,
        retention: &RetentionConfig,
    ) -> rusqlite::Result<Option<ResponsesSession>> {
        let conn = self.conn.lock();
        let row: Option<ResponsesSession> = conn
            .query_row(
                "SELECT id, client_id, previous_response_id, model,
                        created_at, last_activity, store, metadata_json,
                        messages_json, tools_json, final_response_json
                   FROM responses_sessions WHERE id = ?1",
                params![id],
                |r| {
                    Ok(ResponsesSession {
                        id: r.get(0)?,
                        client_id: r.get(1)?,
                        previous_response_id: r.get(2)?,
                        model: r.get(3)?,
                        created_at: r.get(4)?,
                        last_activity: r.get(5)?,
                        store: r.get::<_, i64>(6)? != 0,
                        metadata_json: r.get(7)?,
                        messages_json: r.get(8)?,
                        tools_json: r.get(9)?,
                        final_response_json: r.get(10)?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(e)
                }
            })?;

        let Some(session) = row else {
            return Ok(None);
        };

        let cutoff = Utc::now().timestamp() - retention.active_window_hours * 3600;
        if session.last_activity < cutoff {
            debug!(
                "Responses session {} is cold (last_activity {} < cutoff {}), treating as expired",
                id, session.last_activity, cutoff
            );
            return Ok(None);
        }
        Ok(Some(session))
    }

    /// Delete rows whose `last_activity` is older than
    /// `retention_days`. Returns the number of rows pruned.
    pub fn sweep_expired(&self, retention: &RetentionConfig) -> rusqlite::Result<usize> {
        let cutoff = Utc::now().timestamp() - retention.retention_days * 86_400;
        let conn = self.conn.lock();
        let pruned = conn.execute(
            "DELETE FROM responses_sessions WHERE last_activity < ?1",
            params![cutoff],
        )?;
        if pruned > 0 {
            info!(
                "Responses session sweeper removed {} expired row(s)",
                pruned
            );
        }
        Ok(pruned)
    }

    /// Count of persisted sessions — handy for tests/metrics.
    pub fn len(&self) -> rusqlite::Result<i64> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM responses_sessions", [], |r| {
            r.get::<_, i64>(0)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.len().unwrap_or(0) == 0
    }
}

/// Helper: serialize the messages + tools slice to JSON strings for
/// storage. Fail-safe: if serialization breaks, log + return empty.
pub fn serialize_history(
    messages: &[ChatMessage],
    tools: Option<&[Tool]>,
) -> (String, Option<String>) {
    let messages_json = serde_json::to_string(messages).unwrap_or_else(|e| {
        warn!("Failed to serialize messages: {}", e);
        "[]".to_string()
    });
    let tools_json = tools.map(|t| {
        serde_json::to_string(t).unwrap_or_else(|e| {
            warn!("Failed to serialize tools: {}", e);
            "[]".to_string()
        })
    });
    (messages_json, tools_json)
}

/// Deserialize messages + tools back out of their JSON blobs. Silently
/// drops corrupted blobs in favor of empty collections so a garbage
/// row never brings down a turn.
pub fn deserialize_history(
    messages_json: &str,
    tools_json: Option<&str>,
) -> (Vec<ChatMessage>, Option<Vec<Tool>>) {
    let messages: Vec<ChatMessage> = serde_json::from_str(messages_json).unwrap_or_else(|e| {
        warn!("Failed to deserialize messages blob: {}", e);
        Vec::new()
    });
    let tools = tools_json.and_then(|s| {
        serde_json::from_str::<Vec<Tool>>(s)
            .map_err(|e| warn!("Failed to deserialize tools blob: {}", e))
            .ok()
    });
    (messages, tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, messages: &str) -> ResponsesSession {
        let now = Utc::now().timestamp();
        ResponsesSession {
            id: id.into(),
            client_id: "client-1".into(),
            previous_response_id: None,
            model: "gpt-4o".into(),
            created_at: now,
            last_activity: now,
            store: true,
            metadata_json: None,
            messages_json: messages.into(),
            tools_json: None,
            final_response_json: None,
        }
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let store = ResponsesSessionStore::in_memory().unwrap();
        let s = sample("resp_1", r#"[{"role":"user","content":"hi"}]"#);
        store.insert(&s).unwrap();
        let out = store
            .get_active("resp_1", &RetentionConfig::default())
            .unwrap()
            .expect("row exists");
        assert_eq!(out.id, "resp_1");
        assert_eq!(out.messages_json, s.messages_json);
    }

    #[test]
    fn cold_session_returns_none() {
        let store = ResponsesSessionStore::in_memory().unwrap();
        let mut s = sample("resp_cold", "[]");
        // Last activity 2 days ago; active window is 24h.
        s.last_activity = Utc::now().timestamp() - 2 * 86_400;
        store.insert(&s).unwrap();
        let out = store
            .get_active("resp_cold", &RetentionConfig::default())
            .unwrap();
        assert!(out.is_none(), "cold row must be treated as expired");
    }

    #[test]
    fn sweep_expired_prunes_old_rows() {
        let store = ResponsesSessionStore::in_memory().unwrap();
        let mut ancient = sample("resp_old", "[]");
        ancient.last_activity = Utc::now().timestamp() - 60 * 86_400;
        let mut fresh = sample("resp_new", "[]");
        fresh.last_activity = Utc::now().timestamp();
        store.insert(&ancient).unwrap();
        store.insert(&fresh).unwrap();

        let pruned = store.sweep_expired(&RetentionConfig::default()).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn history_serialization_roundtrips() {
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: lr_providers::ChatMessageContent::Text("hi".into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }];
        let (mj, tj) = serialize_history(&msgs, None);
        assert!(tj.is_none());
        let (back, _) = deserialize_history(&mj, None);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].role, "user");
    }
}
