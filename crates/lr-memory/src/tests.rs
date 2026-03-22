#[cfg(test)]
mod tests {
    use std::time::Duration;

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
        let messages2: Vec<(&str, &str)> =
            vec![("system", "You are helpful"), ("user", "Different topic")];
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
            .create_session_file(&sessions_dir, "test-session")
            .await
            .unwrap();
        assert!(path.exists());

        // Append exchange
        let ts = "2026-03-20T01:08:39+00:00";
        writer
            .append_exchange(
                &path,
                "What is Rust?",
                "Rust is a systems programming language.",
                ts,
            )
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "<user timestamp=\"2026-03-20T01:08:39+00:00\">\nWhat is Rust?\n</user>\n<assistant>\nRust is a systems programming language.\n</assistant>\n"
        );
    }

    #[tokio::test]
    async fn transcript_format_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let writer = TranscriptWriter::new();
        let path = writer
            .create_session_file(&sessions_dir, "87286ef5-abcd-1234")
            .await
            .unwrap();

        let ts1 = "2026-03-20T01:08:39+00:00";
        let ts2 = "2026-03-20T01:09:15+00:00";

        writer
            .append_exchange(
                &path,
                "recall a past convo",
                "I'd be happy to help! Here's some **markdown**:\n\n## Search Results\n- Result 1",
                ts1,
            )
            .await
            .unwrap();

        writer
            .append_exchange(&path, "tell me more", "Sure! Here are the details...", ts2)
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();

        let expected = "\
<user timestamp=\"2026-03-20T01:08:39+00:00\">
recall a past convo
</user>
<assistant>
I'd be happy to help! Here's some **markdown**:

## Search Results
- Result 1
</assistant>
<user timestamp=\"2026-03-20T01:09:15+00:00\">
tell me more
</user>
<assistant>
Sure! Here are the details...
</assistant>
";

        assert_eq!(content, expected);
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

        // Create a summary in archive (should be deleted on restore)
        let summary_file = archive_dir.join("test-session-summary.md");
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
    // MemoryService FTS5 tests
    // ========================================================================

    #[test]
    fn memory_service_ensure_client_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();
        assert!(client_dir.join("sessions").exists());
        assert!(client_dir.join("archive").exists());
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
    fn memory_service_index_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        // Index some content
        svc.index_transcript("test-client", "session-1", "We decided to use PostgreSQL for the auth service. MySQL had connection pooling issues under load.")
            .unwrap();

        // Search for it
        let results = svc.search("test-client", "PostgreSQL auth", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("PostgreSQL"));
    }

    #[test]
    fn memory_service_search_empty_client() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let results = svc.search("nonexistent", "test", 5).unwrap();
        assert!(results.is_empty());
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

    #[test]
    fn memory_service_persistent_store() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();

        // Index with first service instance
        {
            let svc = crate::MemoryService::new(config.clone(), dir.path().to_path_buf());
            svc.index_transcript(
                "test-client",
                "session-1",
                "Rust is a systems programming language focused on safety and performance.",
            )
            .unwrap();
        }

        // Search with new service instance (simulates app restart)
        {
            let svc = crate::MemoryService::new(config, dir.path().to_path_buf());
            let results = svc.search("test-client", "Rust safety", 5).unwrap();
            assert!(!results.is_empty());
        }
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

    // ========================================================================
    // Compaction visibility tests
    // ========================================================================

    #[test]
    fn active_session_path_returns_none_for_unknown_client() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        assert!(mgr.active_session_path("unknown").is_none());
    }

    #[test]
    fn active_session_path_returns_path_for_active_session() {
        let mgr = SessionManager::new(make_config(3600, 28800));
        let dir = std::path::PathBuf::from("/tmp/test-sessions");
        let (_, path, _) = mgr.get_or_create_session("client-1", &dir);
        assert_eq!(mgr.active_session_path("client-1"), Some(path));
    }

    #[test]
    fn compaction_stats_empty_client() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let stats = svc.get_compaction_stats("no-such-client").unwrap();
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.pending_compaction, 0);
        assert_eq!(stats.archived_sessions, 0);
        assert_eq!(stats.indexed_sources, 0);
        assert_eq!(stats.total_lines, 0);
    }

    #[test]
    fn compaction_stats_counts_session_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();

        // Create session files (simulating expired sessions)
        std::fs::write(client_dir.join("sessions/aaa.md"), "content a").unwrap();
        std::fs::write(client_dir.join("sessions/bbb.md"), "content b").unwrap();

        // Create archive files
        std::fs::write(client_dir.join("archive/ccc.md"), "content c").unwrap();

        let stats = svc.get_compaction_stats("test-client").unwrap();
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.pending_compaction, 2); // 2 session files, 0 active
        assert_eq!(stats.archived_sessions, 1);
    }

    #[test]
    fn compaction_stats_excludes_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();

        // Create an active session via session manager
        let (_, active_path, _) = svc
            .session_manager
            .get_or_create_session("test-client", &client_dir.join("sessions"));

        // Write the active session file to disk
        std::fs::write(&active_path, "active session").unwrap();

        // Also create an expired (non-active) session file
        std::fs::write(client_dir.join("sessions/expired.md"), "expired").unwrap();

        let stats = svc.get_compaction_stats("test-client").unwrap();
        assert_eq!(stats.active_sessions, 1);
        assert_eq!(stats.pending_compaction, 1); // only the expired one
    }

    #[tokio::test]
    async fn force_compact_moves_expired_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();
        let sessions_dir = client_dir.join("sessions");

        // Create an active session
        let (_, active_path, _) = svc
            .session_manager
            .get_or_create_session("test-client", &sessions_dir);
        std::fs::write(&active_path, "active content").unwrap();

        // Create expired session files
        std::fs::write(sessions_dir.join("expired1.md"), "expired 1").unwrap();
        std::fs::write(sessions_dir.join("expired2.md"), "expired 2").unwrap();

        let result = svc.force_compact("test-client", |_, _| {}).await.unwrap();
        assert_eq!(result.archived_count, 2);
        assert_eq!(result.summarized_count, 0);

        // Active session should still be in sessions/
        assert!(active_path.exists());

        // Expired sessions should be in archive/
        assert!(client_dir.join("archive/expired1.md").exists());
        assert!(client_dir.join("archive/expired2.md").exists());

        // Verify updated stats
        let stats = svc.get_compaction_stats("test-client").unwrap();
        assert_eq!(stats.active_sessions, 1);
        assert_eq!(stats.pending_compaction, 0);
        assert_eq!(stats.archived_sessions, 2);
    }

    #[test]
    fn reindex_rebuilds_fts5_from_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();

        // Create session and archive files with content
        std::fs::write(
            client_dir.join("sessions/s1.md"),
            "PostgreSQL is the database we chose for auth.",
        )
        .unwrap();
        std::fs::write(
            client_dir.join("archive/a1.md"),
            "Redis is used for caching session tokens.",
        )
        .unwrap();

        // Reindex from files
        let mut progress_calls = Vec::new();
        let count = svc
            .reindex("test-client", |current, total| {
                progress_calls.push((current, total));
            })
            .unwrap();

        assert_eq!(count, 2);
        // Should have initial (0, 2) + one per file
        assert_eq!(progress_calls.len(), 3);
        assert_eq!(progress_calls[0], (0, 2));
        assert_eq!(progress_calls[2], (2, 2));

        // Search should find content from both dirs
        let results = svc.search("test-client", "PostgreSQL", 5).unwrap();
        assert!(!results.is_empty());

        let results = svc.search("test-client", "Redis caching", 5).unwrap();
        assert!(!results.is_empty());
    }

    // ========================================================================
    // LLM Compaction tests
    // ========================================================================

    /// Mock CompactionLlm that returns a fixed summary.
    struct MockLlm {
        summary: String,
    }

    impl MockLlm {
        fn new(summary: &str) -> Self {
            Self {
                summary: summary.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::compaction::CompactionLlm for MockLlm {
        async fn summarize(&self, _model: &str, _transcript: &str) -> Result<String, String> {
            Ok(self.summary.clone())
        }
    }

    /// Mock CompactionLlm that always fails.
    struct FailingLlm;

    #[async_trait::async_trait]
    impl crate::compaction::CompactionLlm for FailingLlm {
        async fn summarize(&self, _model: &str, _transcript: &str) -> Result<String, String> {
            Err("LLM unavailable".to_string())
        }
    }

    #[tokio::test]
    async fn compact_session_with_llm_creates_summary() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archive_dir).unwrap();

        let session_path = sessions_dir.join("test-session.md");
        std::fs::write(&session_path, "full conversation transcript here").unwrap();

        let llm = MockLlm::new("# Summary\nKey points discussed.");
        let outcome = crate::compaction::compact_session(
            &session_path,
            &archive_dir,
            Some(&llm),
            Some("test/model"),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            crate::compaction::CompactionOutcome::ArchivedAndSummarized
        );
        // Raw file moved to archive
        assert!(!session_path.exists());
        assert!(archive_dir.join("test-session.md").exists());
        // Summary file created
        assert!(archive_dir.join("test-session-summary.md").exists());
        let summary = std::fs::read_to_string(archive_dir.join("test-session-summary.md")).unwrap();
        assert!(summary.contains("Key points discussed"));
    }

    #[tokio::test]
    async fn compact_session_without_llm_archives_only() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archive_dir).unwrap();

        let session_path = sessions_dir.join("test-session.md");
        std::fs::write(&session_path, "conversation content").unwrap();

        let outcome = crate::compaction::compact_session(&session_path, &archive_dir, None, None)
            .await
            .unwrap();

        assert_eq!(outcome, crate::compaction::CompactionOutcome::ArchivedOnly);
        assert!(archive_dir.join("test-session.md").exists());
        assert!(!archive_dir.join("test-session-summary.md").exists());
    }

    #[tokio::test]
    async fn compact_session_llm_failure_still_archives() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archive_dir).unwrap();

        let session_path = sessions_dir.join("test-session.md");
        std::fs::write(&session_path, "conversation content").unwrap();

        let llm = FailingLlm;
        let outcome = crate::compaction::compact_session(
            &session_path,
            &archive_dir,
            Some(&llm),
            Some("test/model"),
        )
        .await
        .unwrap();

        // Should archive but not summarize
        assert_eq!(outcome, crate::compaction::CompactionOutcome::ArchivedOnly);
        assert!(archive_dir.join("test-session.md").exists());
        assert!(!archive_dir.join("test-session-summary.md").exists());
    }

    #[tokio::test]
    async fn recompact_session_creates_summary() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();

        // Create raw transcript in archive
        std::fs::write(archive_dir.join("abc123.md"), "original transcript").unwrap();

        let llm = MockLlm::new("# Recompacted summary");
        crate::compaction::recompact_session("abc123", &archive_dir, &llm, "test/model")
            .await
            .unwrap();

        // Summary file created
        assert!(archive_dir.join("abc123-summary.md").exists());
        let summary = std::fs::read_to_string(archive_dir.join("abc123-summary.md")).unwrap();
        assert!(summary.contains("Recompacted summary"));
        // Raw file still exists
        assert!(archive_dir.join("abc123.md").exists());
    }

    #[tokio::test]
    async fn recompact_session_overwrites_existing_summary() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();

        std::fs::write(archive_dir.join("abc123.md"), "original transcript").unwrap();
        std::fs::write(archive_dir.join("abc123-summary.md"), "old summary").unwrap();

        let llm = MockLlm::new("# New summary");
        crate::compaction::recompact_session("abc123", &archive_dir, &llm, "test/model")
            .await
            .unwrap();

        let summary = std::fs::read_to_string(archive_dir.join("abc123-summary.md")).unwrap();
        assert!(summary.contains("New summary"));
        assert!(!summary.contains("old summary"));
    }

    #[test]
    fn compaction_stats_counts_summaries_separately() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();

        // Create archive files: 2 raw + 1 summary
        std::fs::write(client_dir.join("archive/aaa.md"), "raw 1").unwrap();
        std::fs::write(client_dir.join("archive/bbb.md"), "raw 2").unwrap();
        std::fs::write(client_dir.join("archive/aaa-summary.md"), "summary 1").unwrap();

        let stats = svc.get_compaction_stats("test-client").unwrap();
        assert_eq!(stats.archived_sessions, 2); // only raw files
        assert_eq!(stats.summarized_sessions, 1); // only summary files
    }

    #[test]
    fn reindex_prefers_summary_over_raw_in_archive() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();

        // Archive with both raw and summary
        std::fs::write(
            client_dir.join("archive/abc.md"),
            "Raw: PostgreSQL was chosen for authentication storage.",
        )
        .unwrap();
        std::fs::write(
            client_dir.join("archive/abc-summary.md"),
            "Summary: Uses PostgreSQL for auth. Key decision: chose SQL over NoSQL.",
        )
        .unwrap();

        // Archive with only raw (no summary)
        std::fs::write(
            client_dir.join("archive/def.md"),
            "Raw: Redis is used for session caching.",
        )
        .unwrap();

        let count = svc.reindex("test-client", |_, _| {}).unwrap();
        // Should index: abc-summary + def (skipping abc raw)
        assert_eq!(count, 2);

        // Search for content in summary (should find)
        let results = svc.search("test-client", "Key decision SQL", 5).unwrap();
        assert!(!results.is_empty());

        // Search for content in raw-only file (should find)
        let results = svc
            .search("test-client", "Redis session caching", 5)
            .unwrap();
        assert!(!results.is_empty());

        // Search for content only in raw file that has a summary (should NOT find)
        let results = svc
            .search("test-client", "authentication storage", 5)
            .unwrap();
        assert!(
            results.is_empty(),
            "Raw file should not be indexed when summary exists"
        );
    }

    #[tokio::test]
    async fn force_compact_with_llm_summarizes() {
        let dir = tempfile::tempdir().unwrap();
        let config = lr_config::MemoryConfig::default();
        let svc = crate::MemoryService::new(config, dir.path().to_path_buf());

        let client_dir = svc.ensure_client_dir("test-client").unwrap();
        let sessions_dir = client_dir.join("sessions");

        // Create expired session
        std::fs::write(
            sessions_dir.join("expired1.md"),
            "conversation about API design",
        )
        .unwrap();

        // Set up compaction LLM
        let llm = std::sync::Arc::new(MockLlm::new("# API Design Summary"));
        svc.set_compaction_llm(llm);

        // Enable compaction with a model
        let mut config = svc.config();
        config.compaction_enabled = true;
        config.compaction_model = Some("test/model".to_string());
        svc.update_config(config);

        let result = svc.force_compact("test-client", |_, _| {}).await.unwrap();
        assert_eq!(result.archived_count, 1);
        assert_eq!(result.summarized_count, 1);

        // Both raw and summary should exist in archive
        assert!(client_dir.join("archive/expired1.md").exists());
        assert!(client_dir.join("archive/expired1-summary.md").exists());
    }
}
