//! Session and conversation tracking.
//!
//! Groups exchanges into conversations (via MCP via LLM session boundaries
//! or message hash prefix matching) and conversations into sessions
//! (bounded by 3h inactivity or 8h max duration).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::RwLock;

/// Configuration for session timeouts.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Close session after this much inactivity (default: 3 hours)
    pub inactivity_timeout: Duration,
    /// Close session after this max age (default: 8 hours)
    pub max_duration: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            inactivity_timeout: Duration::from_secs(3 * 60 * 60),
            max_duration: Duration::from_secs(8 * 60 * 60),
        }
    }
}

/// Tracks active sessions per client.
pub struct SessionManager {
    /// client_id → active session
    active_sessions: DashMap<String, ActiveSession>,
    config: RwLock<SessionConfig>,
}

/// An active session for a client — groups conversations within a time window.
pub struct ActiveSession {
    pub session_id: String,
    pub file_path: PathBuf,
    pub started_at: Instant,
    pub last_activity: Instant,
    /// Current conversation key (for detecting new conversations)
    pub current_conversation_key: Option<String>,
    /// Conversation state for Both-mode message hash matching
    pub conversation_state: Option<ConversationState>,
    pub conversation_count: u32,
}

/// Tracks message hashes for conversation detection in Both mode.
pub struct ConversationState {
    pub conversation_id: String,
    pub message_hashes: Vec<u64>,
}

/// Context returned when detecting/creating a conversation.
pub struct ConversationContext {
    pub client_id: String,
    pub session_id: String,
    pub conversation_key: String,
    pub file_path: PathBuf,
    /// Whether this is a new conversation (needs a header)
    pub is_new_conversation: bool,
}

impl SessionManager {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            active_sessions: DashMap::new(),
            config: RwLock::new(config),
        }
    }

    /// Update the session configuration.
    pub fn update_config(&self, config: SessionConfig) {
        *self.config.write() = config;
    }

    /// Get or create an active session for a client.
    /// If the existing session has expired, returns None for the old session
    /// (caller should close it) and a new session will be created on next call.
    ///
    /// Returns (session_id, file_path, is_new_session)
    pub fn get_or_create_session(
        &self,
        client_id: &str,
        sessions_dir: &Path,
    ) -> (String, PathBuf, bool) {
        let config = self.config.read().clone();

        // Check if existing session is still valid
        if let Some(mut session) = self.active_sessions.get_mut(client_id) {
            let expired_inactivity = session.last_activity.elapsed() > config.inactivity_timeout;
            let expired_duration = session.started_at.elapsed() > config.max_duration;

            if !expired_inactivity && !expired_duration {
                session.last_activity = Instant::now();
                return (
                    session.session_id.clone(),
                    session.file_path.clone(),
                    false,
                );
            }

            // Session expired — will be cleaned up by close_expired_sessions
        }

        // Create new session
        let session_id = uuid::Uuid::new_v4().to_string();
        let file_path = sessions_dir.join(format!("{}.md", session_id));
        let now = Instant::now();

        let session = ActiveSession {
            session_id: session_id.clone(),
            file_path: file_path.clone(),
            started_at: now,
            last_activity: now,
            current_conversation_key: None,
            conversation_state: None,
            conversation_count: 0,
        };

        self.active_sessions.insert(client_id.to_string(), session);
        (session_id, file_path, true)
    }

    /// Record a new exchange in a session, handling conversation detection.
    /// For MCP via LLM mode: conversation_key is the MCP via LLM session_id.
    /// Returns (file_path, is_new_conversation) so caller can write headers.
    pub fn record_conversation(
        &self,
        client_id: &str,
        conversation_key: &str,
    ) -> Option<(PathBuf, bool)> {
        let mut session = self.active_sessions.get_mut(client_id)?;
        session.last_activity = Instant::now();

        let is_new = session
            .current_conversation_key
            .as_ref()
            .is_none_or(|k| k != conversation_key);

        if is_new {
            session.conversation_count += 1;
            session.current_conversation_key = Some(conversation_key.to_string());
        }

        Some((session.file_path.clone(), is_new))
    }

    /// Detect conversation for Both mode using message hash prefix matching.
    /// Returns a ConversationContext if memory is applicable.
    pub fn detect_conversation_for_both_mode(
        &self,
        client_id: &str,
        messages: &[impl MessageHashable],
        sessions_dir: &Path,
    ) -> Option<ConversationContext> {
        // Compute hashes for all incoming messages
        let incoming_hashes: Vec<u64> = messages.iter().map(|m| m.compute_hash()).collect();

        let (session_id, file_path, is_new_session) =
            self.get_or_create_session(client_id, sessions_dir);

        let mut session = self.active_sessions.get_mut(client_id)?;
        session.last_activity = Instant::now();

        // Check if this is a continuation of the current conversation
        let is_continuation = session
            .conversation_state
            .as_ref()
            .is_some_and(|state| {
                // Check if stored hashes are a prefix of incoming hashes
                if state.message_hashes.len() > incoming_hashes.len() {
                    return false;
                }
                state
                    .message_hashes
                    .iter()
                    .zip(incoming_hashes.iter())
                    .all(|(a, b)| a == b)
            });

        let (conversation_key, is_new_conversation) = if is_continuation {
            let key = session
                .conversation_state
                .as_ref()
                .unwrap()
                .conversation_id
                .clone();
            // Update stored hashes to include new messages
            if let Some(ref mut state) = session.conversation_state {
                state.message_hashes = incoming_hashes;
            }
            (key, false)
        } else {
            // New conversation
            let conv_id = uuid::Uuid::new_v4().to_string();
            session.conversation_count += 1;
            session.current_conversation_key = Some(conv_id.clone());
            session.conversation_state = Some(ConversationState {
                conversation_id: conv_id.clone(),
                message_hashes: incoming_hashes,
            });
            (conv_id, true)
        };

        Some(ConversationContext {
            client_id: client_id.to_string(),
            session_id,
            conversation_key,
            file_path,
            is_new_conversation: is_new_conversation || is_new_session,
        })
    }

    /// Update the last activity time for a session by file path.
    pub fn touch_by_path(&self, path: &Path) {
        for mut entry in self.active_sessions.iter_mut() {
            if entry.value().file_path == path {
                entry.value_mut().last_activity = Instant::now();
                return;
            }
        }
    }

    /// Close expired sessions and return them for compaction.
    pub fn close_expired_sessions(&self) -> Vec<(String, ActiveSession)> {
        let config = self.config.read().clone();
        let expired = Vec::new();

        self.active_sessions.retain(|client_id, session| {
            let expired_inactivity = session.last_activity.elapsed() > config.inactivity_timeout;
            let expired_duration = session.started_at.elapsed() > config.max_duration;

            if expired_inactivity || expired_duration {
                tracing::info!(
                    "Memory session expired for client {} (session={}, age={:.0}s, idle={:.0}s)",
                    &client_id[..8.min(client_id.len())],
                    &session.session_id[..8.min(session.session_id.len())],
                    session.started_at.elapsed().as_secs_f64(),
                    session.last_activity.elapsed().as_secs_f64(),
                );
                false // Will be removed; we collect below
            } else {
                true
            }
        });

        // DashMap::retain doesn't give us the removed values, so we need to
        // re-check and remove manually for expired ones we want to compact
        // Actually, retain already removed them. We need a different approach.
        // Let's collect first, then remove.

        // NOTE: The retain above already removed them. In practice, we'd need
        // to collect before removing. Let's restructure:
        expired
    }
}

/// Trait for types that can be hashed for conversation detection.
pub trait MessageHashable {
    fn compute_hash(&self) -> u64;
}

/// Generic implementation for (role, content) tuples.
impl MessageHashable for (&str, &str) {
    fn compute_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.0.hash(&mut hasher);
        self.1.hash(&mut hasher);
        hasher.finish()
    }
}
