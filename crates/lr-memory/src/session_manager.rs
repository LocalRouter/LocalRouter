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
use rand::seq::SliceRandom;

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
    pub file_path: PathBuf,
    /// Memory folder slug (for resolving client dir on session expiry)
    pub memory_folder: String,
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
    pub conversation_key: String,
    pub file_path: PathBuf,
    /// Whether this is a new conversation (needs a header)
    pub is_new_conversation: bool,
}

/// Slugify a content string for use in filenames.
/// "What database should we use?" → "what-database-should-we-use"
/// Truncates at approximately `max_len` chars on a word boundary.
pub fn slugify_content(text: &str, max_len: usize) -> String {
    let mut result = String::new();
    let mut last_was_hyphen = true; // trim leading hyphens
    for c in text.chars() {
        if c.is_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            result.push('-');
            last_was_hyphen = true;
        }
        if result.len() >= max_len {
            break;
        }
    }
    // Trim trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }
    // Try to truncate at last hyphen for a clean word boundary
    if result.len() >= max_len {
        if let Some(pos) = result.rfind('-') {
            result.truncate(pos);
        }
    }
    result
}

/// Generate a random 5-character alphanumeric suffix.
fn random_suffix() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..5)
        .map(|_| *CHARS.choose(&mut rng).unwrap() as char)
        .collect()
}

/// Generate a session filename stem: `{timestamp}-{content-slug}-{random}`
/// Example: `2026-03-22T14-30-00-explain-how-auth-works-x7k2m`
pub fn generate_session_file_stem(content_hint: &str) -> String {
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    let slug = slugify_content(content_hint, 50);
    let suffix = random_suffix();
    if slug.is_empty() {
        format!("{}-{}", timestamp, suffix)
    } else {
        format!("{}-{}-{}", timestamp, slug, suffix)
    }
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
    /// `content_hint` is used to generate a human-readable filename for new sessions
    /// (e.g., the first user message). Ignored for existing sessions.
    ///
    /// `memory_folder` is the slug folder name, stored on the session for later use
    /// by the session monitor when constructing archive paths.
    ///
    /// Returns (file_path, is_new_session)
    pub fn get_or_create_session(
        &self,
        client_id: &str,
        active_dir: &Path,
        content_hint: &str,
        memory_folder: &str,
    ) -> (PathBuf, bool) {
        let config = self.config.read().clone();

        // Check if existing session is still valid
        if let Some(mut session) = self.active_sessions.get_mut(client_id) {
            let expired_inactivity = session.last_activity.elapsed() > config.inactivity_timeout;
            let expired_duration = session.started_at.elapsed() > config.max_duration;

            if !expired_inactivity && !expired_duration {
                session.last_activity = Instant::now();
                return (session.file_path.clone(), false);
            }

            // Drop the mutable ref before removing
            drop(session);
            // Remove expired session to prevent duplicates
            self.active_sessions.remove(client_id);
        }

        // Create new session with human-readable filename
        let file_stem = generate_session_file_stem(content_hint);
        let file_path = active_dir.join(format!("{}.md", file_stem));
        let now = Instant::now();

        let session = ActiveSession {
            file_path: file_path.clone(),
            memory_folder: memory_folder.to_string(),
            started_at: now,
            last_activity: now,
            current_conversation_key: None,
            conversation_state: None,
            conversation_count: 0,
        };

        self.active_sessions.insert(client_id.to_string(), session);
        (file_path, true)
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
        active_dir: &Path,
        content_hint: &str,
        memory_folder: &str,
    ) -> Option<ConversationContext> {
        // Compute hashes for all incoming messages
        let incoming_hashes: Vec<u64> = messages.iter().map(|m| m.compute_hash()).collect();

        let (file_path, is_new_session) =
            self.get_or_create_session(client_id, active_dir, content_hint, memory_folder);

        let mut session = self.active_sessions.get_mut(client_id)?;
        session.last_activity = Instant::now();

        // Check if this is a continuation of the current conversation
        let is_continuation = session.conversation_state.as_ref().is_some_and(|state| {
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

    /// Return the file path of the active session for a client, if any.
    pub fn active_session_path(&self, client_id: &str) -> Option<PathBuf> {
        self.active_sessions
            .get(client_id)
            .map(|s| s.file_path.clone())
    }

    /// Force-close any active session for the given client.
    pub fn force_close(&self, client_id: &str) {
        self.active_sessions.remove(client_id);
    }

    /// Close expired sessions and return them for compaction.
    /// Collects expired session keys first, then removes them one by one
    /// so we can return the removed values.
    pub fn close_expired_sessions(&self) -> Vec<(String, ActiveSession)> {
        let config = self.config.read().clone();

        // First pass: collect keys of expired sessions
        let expired_keys: Vec<String> = self
            .active_sessions
            .iter()
            .filter(|entry| {
                let session = entry.value();
                session.last_activity.elapsed() > config.inactivity_timeout
                    || session.started_at.elapsed() > config.max_duration
            })
            .map(|entry| entry.key().clone())
            .collect();

        // Second pass: remove and collect the expired sessions
        let mut expired = Vec::with_capacity(expired_keys.len());
        for key in expired_keys {
            if let Some((client_id, session)) = self.active_sessions.remove(&key) {
                // Double-check it's still expired (could have been touched between passes)
                let still_expired = session.last_activity.elapsed() > config.inactivity_timeout
                    || session.started_at.elapsed() > config.max_duration;
                if still_expired {
                    let display_name = short_display_id(
                        &session
                            .file_path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    );
                    tracing::info!(
                        "Memory session expired for client {} (session={}, age={:.0}s, idle={:.0}s)",
                        &client_id[..8.min(client_id.len())],
                        display_name,
                        session.started_at.elapsed().as_secs_f64(),
                        session.last_activity.elapsed().as_secs_f64(),
                    );
                    expired.push((client_id, session));
                } else {
                    // Was touched between passes — put it back
                    self.active_sessions.insert(key, session);
                }
            }
        }

        expired
    }
}

/// Get a short display ID from a file stem for logging.
/// "2026-03-22T14-30-00-explain-auth-x7k2m" → "explain-auth-x7k2m"
/// "87286ef5-abcd-1234" → "87286ef5" (legacy UUID)
pub fn short_display_id(file_stem: &str) -> String {
    // Timestamp format: YYYY-MM-DDTHH-MM-SS- = 20 chars
    if file_stem.len() > 20 && file_stem.as_bytes()[19] == b'-' {
        file_stem[20..].to_string()
    } else {
        file_stem[..8.min(file_stem.len())].to_string()
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
