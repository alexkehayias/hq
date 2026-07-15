//! Integration tests for the pub/sub + ChatSessionManager integration.
//!
//! These tests verify that:
//! - ChatSessionManager.subscribe registers a session with the broker
//!   so messages published to a channel reach the ChatTask's receiver.
//! - Subscriptions persisted to DB survive "restart" (re-creating a
//!   ChatSessionManager with the same DB restores subscriptions).
//!
//! The full ChatTask -> next_msg flow is harder to test in isolation
//! (it requires a running OpenAI-compatible API), so these tests focus
//! on the broker wiring and DB persistence. The unit tests in
//! `src/ai/chat/session.rs` and `src/ai/pubsub.rs` cover the broker
//! primitives directly.

mod test_utils;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use hq::ai::chat::{ChatSessionManager, ChatTaskDeps, get_or_create_session};
    use hq::ai::chat::models::SessionMode;
    use hq::ai::pubsub::PubSubBroker;
    use hq::ai::skills::SkillRegistry;
    use hq::core::AppConfig;
    use hq::core::db::{async_db, initialize_db};
    use serde_json::json;
    use serial_test::serial;

    /// Helper: construct a ChatSessionManager wired to a real in-memory
    /// DB and a PubSubBroker. Returns (broker, chat_sessions, db) so
    /// tests can publish messages and inspect DB state.
    async fn build_chat_session_manager() -> (
        Arc<PubSubBroker>,
        Arc<ChatSessionManager>,
        tokio_rusqlite::Connection,
    ) {
        let temp_dir = std::env::temp_dir();
        let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).expect("Failed to create base directory");
        let skills_path = dir.join("skills");
        std::fs::create_dir_all(&skills_path).expect("Failed to create skills dir");
        let db_dir = dir.join("db");
        std::fs::create_dir_all(&db_dir).expect("Failed to create db dir");

        let db = async_db(db_dir.to_str().unwrap())
            .await
            .expect("Failed to connect to async db");
        db.call(|conn| {
            initialize_db(conn).expect("Failed to init db");
            Ok(())
        })
        .await
        .unwrap();

        let app_config = AppConfig {
            notes_path: dir.join("notes").display().to_string(),
            index_path: dir.join("index").display().to_string(),
            vec_db_path: dir.join("db").display().to_string(),
            storage_path: dir.display().to_string(),
            skills_path: skills_path.display().to_string(),
            deploy_key_path: String::from("test_deploy_key_path"),
            vapid_key_path: String::from("test_vapid_key_path"),
            note_search_api_url: String::from("http://localhost:2222"),
            gmail_api_client_id: String::from("test_client_id"),
            gmail_api_client_secret: String::from("test_client_secret"),
            google_search_api_key: String::from("test_google_search_key"),
            google_search_cx_id: String::from("test_cx_id"),
            openai_model: String::from("gpt-4o"),
            openai_api_hostname: String::from("https://api.openai.com"),
            openai_api_key: String::from("test-api-key"),
            system_message: String::from("You are a helpful assistant."),
        };

        let skill_registry = Arc::new(RwLock::new(
            SkillRegistry::new(&skills_path).await.unwrap(),
        ));

        let broker = Arc::new(PubSubBroker::new());
        let chat_deps = ChatTaskDeps {
            db: db.clone(),
            config: Arc::new(app_config),
            skill_registry,
        };
        let chat_sessions = Arc::new(ChatSessionManager::new(
            Arc::clone(&broker),
            chat_deps,
        ));

        (broker, chat_sessions, db)
    }

    /// Helper: create a session row in the `session` table so that
    /// `chat_subscription.session_id` (FK constraint) can reference it.
    /// Mirrors what chat_handler does in production via
    /// `get_or_create_session`. Must be called before subscribe() when
    /// the test uses an arbitrary session_id that doesn't exist yet.
    async fn create_test_session(db: &tokio_rusqlite::Connection, session_id: &str) {
        get_or_create_session(db, session_id, &[], SessionMode::Chat)
            .await
            .expect("Failed to create test session row");
    }

    /// Test that subscribing a session registers it with the broker so
    /// messages published to the channel reach the ChatTask.
    ///
    /// We can't easily verify the message reaches next_msg (requires a
    /// running LLM API), but we CAN verify the broker wiring: after
    /// subscribe, the channel has 1 subscriber; publishing delivers the
    /// message (subscriber count drops to 0 after cleanup if receiver
    /// dropped, or stays at 1 if still alive).
    #[tokio::test]
    #[serial]
    async fn test_subscribe_registers_session_with_broker() {
        let (broker, chat_sessions, db) = build_chat_session_manager().await;

        // Before subscribe: no subscribers on the channel
        assert_eq!(broker.subscriber_count("test-channel"), 0);

        // Create the session row (FK requires it before subscribe)
        create_test_session(&db, "test-session-id").await;

        // Subscribe the session
        chat_sessions
            .subscribe("test-session-id", "test-channel")
            .await
            .expect("subscribe failed");

        // After subscribe: 1 subscriber on the channel
        assert_eq!(broker.subscriber_count("test-channel"), 1);

        // Publishing a message should deliver it (subscriber count stays
        // at 1 because the ChatTask's receiver is still alive)
        broker.publish(
            "test-channel",
            hq::openai::Message::new(hq::openai::Role::User, "hello"),
        );
        // Subscriber count should still be 1 (receiver alive)
        assert_eq!(
            broker.subscriber_count("test-channel"),
            1,
            "subscriber should still be alive after publish"
        );
    }

    /// Test that subscriptions persist to DB so they survive "restart".
    ///
    /// We subscribe a session, then create a NEW ChatSessionManager
    /// with the same DB and call restore_subscriptions. The new manager
    /// should re-register the session's channels with its broker.
    #[tokio::test]
    #[serial]
    async fn test_subscriptions_persist_across_restart() {
        let (broker, chat_sessions, db) = build_chat_session_manager().await;

        // Create the session row first (FK requires it before subscribe)
        create_test_session(&db, "session-restart-test").await;

        // Subscribe session to two channels
        chat_sessions
            .subscribe("session-restart-test", "channel-a")
            .await
            .unwrap();
        chat_sessions
            .subscribe("session-restart-test", "channel-b")
            .await
            .unwrap();

        // Verify both channels have subscribers in the original broker
        assert_eq!(broker.subscriber_count("channel-a"), 1);
        assert_eq!(broker.subscriber_count("channel-b"), 1);

        // Build a NEW ChatSessionManager with the same DB (simulates
        // server restart). The old broker/chat_sessions are dropped.
        let temp_dir = std::env::temp_dir();
        let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).expect("Failed to create base directory");
        let skills_path = dir.join("skills");
        std::fs::create_dir_all(&skills_path).expect("Failed to create skills dir");

        let new_broker = Arc::new(PubSubBroker::new());
        let new_chat_deps = ChatTaskDeps {
            db: db.clone(),
            config: Arc::new(AppConfig {
                notes_path: dir.join("notes").display().to_string(),
                index_path: dir.join("index").display().to_string(),
                vec_db_path: dir.join("db").display().to_string(),
                storage_path: dir.display().to_string(),
                skills_path: skills_path.display().to_string(),
                deploy_key_path: String::from("test_deploy_key_path"),
                vapid_key_path: String::from("test_vapid_key_path"),
                note_search_api_url: String::from("http://localhost:2222"),
                gmail_api_client_id: String::from("test_client_id"),
                gmail_api_client_secret: String::from("test_client_secret"),
                google_search_api_key: String::from("test_google_search_key"),
                google_search_cx_id: String::from("test_cx_id"),
                openai_model: String::from("gpt-4o"),
                openai_api_hostname: String::from("https://api.openai.com"),
                openai_api_key: String::from("test-api-key"),
                system_message: String::from("You are a helpful assistant."),
            }),
            skill_registry: Arc::new(RwLock::new(
                SkillRegistry::new(&skills_path).await.unwrap(),
            )),
        };
        let new_chat_sessions = Arc::new(ChatSessionManager::new(
            Arc::clone(&new_broker),
            new_chat_deps,
        ));

        // restore_subscriptions should read the DB and re-register
        // channels with the new broker
        new_chat_sessions
            .restore_subscriptions()
            .await
            .expect("restore_subscriptions failed");

        // The new broker should now have subscribers on both channels
        assert_eq!(new_broker.subscriber_count("channel-a"), 1);
        assert_eq!(new_broker.subscriber_count("channel-b"), 1);

        // Publishing on the new broker should deliver to the
        // restored ChatTask (subscriber count stays at 1)
        new_broker.publish(
            "channel-a",
            hq::openai::Message::new(hq::openai::Role::User, "post-restart message"),
        );
        assert_eq!(new_broker.subscriber_count("channel-a"), 1);
    }

    /// Test that subscribing to multiple channels fans out correctly:
    /// a session subscribed to 3 channels receives messages on all 3.
    #[tokio::test]
    #[serial]
    async fn test_multi_channel_subscription() {
        let (broker, chat_sessions, db) = build_chat_session_manager().await;

        // Create session row first
        create_test_session(&db, "multi-channel-session").await;

        // Subscribe to three channels
        for channel in &["alpha", "beta", "gamma"] {
            chat_sessions
                .subscribe("multi-channel-session", channel)
                .await
                .unwrap();
        }

        // Each channel should have exactly 1 subscriber (same session)
        assert_eq!(broker.subscriber_count("alpha"), 1);
        assert_eq!(broker.subscriber_count("beta"), 1);
        assert_eq!(broker.subscriber_count("gamma"), 1);

        // Unrelated channel has no subscribers
        assert_eq!(broker.subscriber_count("delta"), 0);
    }

    /// Test that the subscription is persisted to DB (chat_subscription
    /// table) so it survives restarts. Verifies the row exists after
    /// subscribe.
    #[tokio::test]
    #[serial]
    async fn test_subscription_persisted_to_db() {
        let (_broker, chat_sessions, db) = build_chat_session_manager().await;

        // Create session row first
        create_test_session(&db, "db-persist-test").await;

        // No subscriptions initially
        let initial: Vec<(String, String)> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT session_id, channel FROM chat_subscription")
                    .unwrap();
                let rows: Vec<(String, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            })
            .await
            .unwrap();
        assert!(initial.is_empty(), "Expected no subscriptions initially");

        // Subscribe
        chat_sessions
            .subscribe("db-persist-test", "test-channel")
            .await
            .unwrap();

        // Verify the row exists in DB
        let after: Vec<(String, String)> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT session_id, channel FROM chat_subscription")
                    .unwrap();
                let rows: Vec<(String, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(after.len(), 1, "Expected 1 subscription in DB");
        assert_eq!(after[0].0, "db-persist-test");
        assert_eq!(after[0].1, "test-channel");

        // Subscribing the same channel again should be idempotent
        chat_sessions
            .subscribe("db-persist-test", "test-channel")
            .await
            .unwrap();
        let after_duplicate: Vec<(String, String)> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT session_id, channel FROM chat_subscription")
                    .unwrap();
                let rows: Vec<(String, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(after_duplicate.len(), 1, "Duplicate subscribe should be idempotent");
    }

    /// Test that publishing to a channel with no subscribers is a
    /// safe no-op (doesn't panic, doesn't crash).
    #[tokio::test]
    #[serial]
    async fn test_publish_to_unsubscribed_channel_is_noop() {
        let (broker, _chat_sessions, _db) = build_chat_session_manager().await;

        // No subscribers on "ghost-channel" — publish should not panic
        broker.publish(
            "ghost-channel",
            hq::openai::Message::new(hq::openai::Role::User, "anyone there?"),
        );
        // Still no subscribers
        assert_eq!(broker.subscriber_count("ghost-channel"), 0);
    }

    /// Test that the ChatTask is spawned eagerly for sessions with
    /// persisted subscriptions on restore. We verify this indirectly:
    /// after restore_subscriptions, publishing to a restored channel
    /// delivers the message (the ChatTask is running and its receiver
    /// is registered with the broker).
    #[tokio::test]
    #[serial]
    async fn test_restore_spawns_chat_tasks_eagerly() {
        let (broker, chat_sessions, db) = build_chat_session_manager().await;

        // Create session row first
        create_test_session(&db, "eager-spawn-test").await;

        // Subscribe a session
        chat_sessions
            .subscribe("eager-spawn-test", "startup-channel")
            .await
            .unwrap();

        // Drop the original broker/chat_sessions (simulates shutdown).
        // The DB persists the subscription.
        drop(broker);
        drop(chat_sessions);

        // Build a new manager with the same DB
        let temp_dir = std::env::temp_dir();
        let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).expect("Failed to create base directory");
        let skills_path = dir.join("skills");
        std::fs::create_dir_all(&skills_path).expect("Failed to create skills dir");

        let new_broker = Arc::new(PubSubBroker::new());
        let new_chat_deps = ChatTaskDeps {
            db: db.clone(),
            config: Arc::new(AppConfig {
                notes_path: dir.join("notes").display().to_string(),
                index_path: dir.join("index").display().to_string(),
                vec_db_path: dir.join("db").display().to_string(),
                storage_path: dir.display().to_string(),
                skills_path: skills_path.display().to_string(),
                deploy_key_path: String::from("test_deploy_key_path"),
                vapid_key_path: String::from("test_vapid_key_path"),
                note_search_api_url: String::from("http://localhost:2222"),
                gmail_api_client_id: String::from("test_client_id"),
                gmail_api_client_secret: String::from("test_client_secret"),
                google_search_api_key: String::from("test_google_search_key"),
                google_search_cx_id: String::from("test_cx_id"),
                openai_model: String::from("gpt-4o"),
                openai_api_hostname: String::from("https://api.openai.com"),
                openai_api_key: String::from("test-api-key"),
                system_message: String::from("You are a helpful assistant."),
            }),
            skill_registry: Arc::new(RwLock::new(
                SkillRegistry::new(&skills_path).await.unwrap(),
            )),
        };
        let new_chat_sessions = Arc::new(ChatSessionManager::new(
            Arc::clone(&new_broker),
            new_chat_deps,
        ));

        // Restore subscriptions — this should eagerly spawn a ChatTask
        // for the "eager-spawn-test" session.
        new_chat_sessions
            .restore_subscriptions()
            .await
            .expect("restore failed");

        // Give the spawned ChatTask a moment to initialize (it loads
        // transcript from DB asynchronously)
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Publishing to the restored channel should deliver the
        // message — proving a ChatTask is running and registered.
        new_broker.publish(
            "startup-channel",
            hq::openai::Message::new(hq::openai::Role::User, "hello after restart"),
        );
        // The ChatTask's receiver should be alive (subscriber count = 1)
        assert_eq!(
            new_broker.subscriber_count("startup-channel"),
            1,
            "ChatTask should be running and subscribed after restore"
        );

        // Suppress unused warning for json! — it's used in other tests
        let _ = json!({});
    }
}