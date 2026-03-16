#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::cli::SearchResult;
    use crate::session_manager::{MessageHashable, SessionConfig, SessionManager};
    use crate::transcript::TranscriptWriter;

    // ========================================================================
    // SessionManager tests
    // ========================================================================

    fn make_config(inactivity_secs: u64, max_secs: u64) -> SessionConfig {
        SessionConfig {
            inactivity_timeout: Duration::from_secs(inactivity_secs),
            max_duration: Duration::from_secs(max_secs),
        }
    }

    #[test]
    fn session_manager_creates_new_session() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let (id, path, is_new) = mgr.get_or_create_session("client-1", &dir);
        assert!(is_new);
        assert!(!id.is_empty());
        assert!(path.to_string_lossy().contains("client-1") == false); // path is in /tmp/test-sessions
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn session_manager_reuses_active_session() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let (id1, _, is_new1) = mgr.get_or_create_session("client-1", &dir);
        assert!(is_new1);

        let (id2, _, is_new2) = mgr.get_or_create_session("client-1", &dir);
        assert!(!is_new2);
        assert_eq!(id1, id2);
    }

    #[test]
    fn session_manager_isolates_clients() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let (id1, _, _) = mgr.get_or_create_session("client-1", &dir);
        let (id2, _, _) = mgr.get_or_create_session("client-2", &dir);
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_manager_records_conversation() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        // Create session first
        mgr.get_or_create_session("client-1", &dir);

        // First conversation
        let result = mgr.record_conversation("client-1", "conv-1");
        assert!(result.is_some());
        let (_, is_new) = result.unwrap();
        assert!(is_new);

        // Same conversation
        let result = mgr.record_conversation("client-1", "conv-1");
        assert!(result.is_some());
        let (_, is_new) = result.unwrap();
        assert!(!is_new);

        // New conversation
        let result = mgr.record_conversation("client-1", "conv-2");
        assert!(result.is_some());
        let (_, is_new) = result.unwrap();
        assert!(is_new);
    }

    #[test]
    fn session_manager_records_conversation_nonexistent_client() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let result = mgr.record_conversation("nonexistent", "conv-1");
        assert!(result.is_none());
    }

    #[test]
    fn close_expired_sessions_returns_expired() {
        let mgr = SessionManager::new(make_config(0, 28800)); // 0 inactivity = instant expire
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        // Create a session
        mgr.get_or_create_session("client-1", &dir);

        // Wait a tiny bit to ensure elapsed > 0
        std::thread::sleep(Duration::from_millis(10));

        let expired = mgr.close_expired_sessions();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, "client-1");
    }

    #[test]
    fn close_expired_sessions_preserves_active() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        mgr.get_or_create_session("client-1", &dir);

        let expired = mgr.close_expired_sessions();
        assert_eq!(expired.len(), 0);
    }

    #[test]
    fn close_expired_by_max_duration() {
        // Max duration of 0 = instant expire
        let mgr = SessionManager::new(make_config(3600, 0));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        mgr.get_or_create_session("client-1", &dir);
        std::thread::sleep(Duration::from_millis(10));

        let expired = mgr.close_expired_sessions();
        assert_eq!(expired.len(), 1);
    }

    #[test]
    fn touch_by_path_updates_activity() {
        let mgr = SessionManager::new(make_config(1, 28800)); // 1 second inactivity
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let (_, path, _) = mgr.get_or_create_session("client-1", &dir);

        // Wait, but touch before expiry
        std::thread::sleep(Duration::from_millis(500));
        mgr.touch_by_path(&path);

        // Should still be active
        let expired = mgr.close_expired_sessions();
        assert_eq!(expired.len(), 0);
    }

    // ========================================================================
    // MessageHashable tests
    // ========================================================================

    #[test]
    fn message_hash_deterministic() {
        let msg1 = ("user", "Hello");
        let msg2 = ("user", "Hello");
        assert_eq!(msg1.compute_hash(), msg2.compute_hash());
    }

    #[test]
    fn message_hash_differs_by_role() {
        let msg1 = ("user", "Hello");
        let msg2 = ("assistant", "Hello");
        assert_ne!(msg1.compute_hash(), msg2.compute_hash());
    }

    #[test]
    fn message_hash_differs_by_content() {
        let msg1 = ("user", "Hello");
        let msg2 = ("user", "Goodbye");
        assert_ne!(msg1.compute_hash(), msg2.compute_hash());
    }

    // ========================================================================
    // Conversation detection tests
    // ========================================================================

    #[test]
    fn detect_conversation_new_session() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let messages: Vec<(&str, &str)> = vec![("system", "You are helpful"), ("user", "Hi")];
        let ctx = mgr.detect_conversation_for_both_mode("client-1", &messages, &dir);
        assert!(ctx.is_some());
        let ctx = ctx.unwrap();
        assert!(ctx.is_new_conversation);
    }

    #[test]
    fn detect_conversation_continuation() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let messages1: Vec<(&str, &str)> = vec![("system", "You are helpful"), ("user", "Hi")];
        let ctx1 = mgr
            .detect_conversation_for_both_mode("client-1", &messages1, &dir)
            .unwrap();
        assert!(ctx1.is_new_conversation);

        // Same messages + new ones = continuation
        let messages2: Vec<(&str, &str)> = vec![
            ("system", "You are helpful"),
            ("user", "Hi"),
            ("assistant", "Hello!"),
            ("user", "How are you?"),
        ];
        let ctx2 = mgr
            .detect_conversation_for_both_mode("client-1", &messages2, &dir)
            .unwrap();
        assert!(!ctx2.is_new_conversation);
        assert_eq!(ctx1.conversation_key, ctx2.conversation_key);
    }

    #[test]
    fn detect_conversation_new_topic() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        let messages1: Vec<(&str, &str)> = vec![("system", "You are helpful"), ("user", "Hi")];
        let ctx1 = mgr
            .detect_conversation_for_both_mode("client-1", &messages1, &dir)
            .unwrap();

        // Completely different messages = new conversation
        let messages2: Vec<(&str, &str)> = vec![("system", "You are helpful"), ("user", "Different topic")];
        let ctx2 = mgr
            .detect_conversation_for_both_mode("client-1", &messages2, &dir)
            .unwrap();
        assert!(ctx2.is_new_conversation);
        assert_ne!(ctx1.conversation_key, ctx2.conversation_key);
        // But same session
        assert_eq!(ctx1.session_id, ctx2.session_id);
    }

    // ========================================================================
    // TranscriptWriter tests
    // ========================================================================

    #[tokio::test]
    async fn transcript_create_and_append() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let writer = TranscriptWriter::new();

        // Create session file
        let path = writer
            .create_session_file(&sessions_dir, "test-session", "test-client")
            .await
            .unwrap();
        assert!(path.exists());

        // Check frontmatter
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("client_id: test-client"));
        assert!(content.contains("session_id: test-session"));

        // Append conversation header
        writer
            .append_conversation_header(&path, "conv-1", "10:30")
            .await
            .unwrap();

        // Append exchange
        writer
            .append_exchange(&path, "What is Rust?", "Rust is a systems programming language.")
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Conversation conv-1 (10:30)"));
        assert!(content.contains("## User\nWhat is Rust?"));
        assert!(content.contains("## Assistant\nRust is a systems programming language."));
    }

    #[tokio::test]
    async fn transcript_restore_from_archive() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archive_dir).unwrap();

        let writer = TranscriptWriter::new();

        // Create a file in archive
        let archive_file = archive_dir.join("test-session.md");
        std::fs::write(&archive_file, "---\ntest content\n---\n").unwrap();

        // Create a summary in sessions (should be deleted on restore)
        let summary_file = sessions_dir.join("test-session-summary.md");
        std::fs::write(&summary_file, "summary content").unwrap();

        // Restore
        let restored = writer
            .restore_from_archive("test-session", &sessions_dir, &archive_dir)
            .await
            .unwrap();

        assert!(restored.exists());
        assert!(!archive_file.exists());
        assert!(!summary_file.exists());

        let content = std::fs::read_to_string(&restored).unwrap();
        assert!(content.contains("test content"));
    }

    // ========================================================================
    // SearchResult deserialization tests
    // ========================================================================

    #[test]
    fn search_result_deserialize_basic() {
        let json = r#"{"source": "session.md", "content": "Hello world", "score": 0.85}"#;
        let result: SearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.source, "session.md");
        assert_eq!(result.content, "Hello world");
        assert_eq!(result.score, Some(0.85));
    }

    #[test]
    fn search_result_deserialize_with_hash() {
        let json = r###"{"source": "s.md", "content": "test", "chunk_hash": "abc123", "heading": "Title"}"###;
        let result: SearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.chunk_hash, Some("abc123".to_string()));
        assert_eq!(result.heading, Some("Title".to_string()));
    }

    #[test]
    fn search_result_deserialize_minimal() {
        let json = r#"{"source": "s.md", "content": "test"}"#;
        let result: SearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.score, None);
        assert_eq!(result.chunk_hash, None);
        assert_eq!(result.heading, None);
    }

    // ========================================================================
    // MemoryService tests
    // ========================================================================

    #[test]
    fn memory_service_ensure_client_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();
        assert!(client_dir.join("sessions").exists());
        assert!(client_dir.join("archive").exists());
        // No .memsearch.toml — provider is passed via CLI args
        assert!(!client_dir.join(".memsearch.toml").exists());
    }

    #[test]
    fn memory_service_ensure_client_dir_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let dir1 = svc.ensure_client_dir("test-client").unwrap();
        let dir2 = svc.ensure_client_dir("test-client").unwrap();
        assert_eq!(dir1, dir2);
    }

    #[test]
    fn memory_service_uses_onnx_provider_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());
        assert_eq!(svc.cli.provider, "local");
    }

    #[test]
    fn memory_service_uses_ollama_provider_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig {
            embedding: lr_config::MemoryEmbeddingConfig::Ollama {
                provider_id: "my-ollama".to_string(),
                model_name: "nomic-embed-text".to_string(),
            },
            ..Default::default()
        };
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());
        assert_eq!(svc.cli.provider, "ollama");
    }

    #[test]
    fn session_manager_removes_expired_before_creating_new() {
        let mgr = SessionManager::new(make_config(0, 28800)); // 0 inactivity = instant expire
        let dir = std::path::PathBuf::from("/tmp/test-sessions");

        // Create initial session
        let (id1, _, _) = mgr.get_or_create_session("client-1", &dir);

        // Wait for it to expire
        std::thread::sleep(Duration::from_millis(10));

        // Should create a new session (not return expired one)
        let (id2, _, is_new) = mgr.get_or_create_session("client-1", &dir);
        assert!(is_new);
        assert_ne!(id1, id2);

        // The expired session was already removed by get_or_create_session,
        // so close_expired_sessions should find nothing
        std::thread::sleep(Duration::from_millis(10));
        let expired = mgr.close_expired_sessions();
        // The new session also expires instantly (0s TTL), so it gets collected too
        assert!(expired.len() <= 1);
    }

    #[test]
    fn memory_service_update_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        assert_eq!(svc.config().search_top_k, 5);

        let new_config = lr_config::MemoryConfig {
            search_top_k: 10,
            ..Default::default()
        };
        svc.update_config(new_config);
        assert_eq!(svc.config().search_top_k, 10);
    }

    // ========================================================================
    // SessionConfig tests
    // ========================================================================

    #[test]
    fn session_config_default_values() {
        let config = SessionConfig::default();
        assert_eq!(config.inactivity_timeout, Duration::from_secs(3 * 60 * 60));
        assert_eq!(config.max_duration, Duration::from_secs(8 * 60 * 60));
    }
}
