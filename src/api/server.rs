use std::sync::{Arc, RwLock};

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use super::{assets, routes};
use crate::ai::skills::SkillRegistry;
use crate::api::state::AppState;
use crate::channels::default::{register_agent_channels, start_default_channels};
use crate::channels::errors::{ErrorChannelLayer, register as register_error_channel};
use crate::core::{AppConfig, db::async_db};
use crate::jobs::{DailyAgenda, GenerateSessionTitles, GitSync, spawn_periodic_job};
use crate::loops::supervisor::{self as loops, LoopSupervisorConfig};

/// Directives used when `RUST_LOG` is unset.
///
/// The leading `error` directive is global. Without it, `EnvFilter` disables
/// every target that has no explicit directive, so `ERROR`-level events from
/// other crates (and the `log` bridge) would never reach the logs. `hq` and the
/// HTTP stack stay more verbose: axum logs extractor rejections at `TRACE`
/// under `axum::rejection`, hence that target's directive.
fn default_log_filter() -> String {
    format!(
        "error,{}=debug,tower_http=debug,axum::rejection=trace",
        env!("CARGO_CRATE_NAME")
    )
}

pub fn app(shared_state: Arc<RwLock<AppState>>) -> Router {
    let cors = CorsLayer::permissive();

    let router = Router::new()
        // API routes
        .nest("/api", routes::router());

    // Static assets: embedded in the binary (prod), or served from disk (dev).
    let router = assets::attach_assets(router);

    router
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(Arc::clone(&shared_state))
}

// Run the server
#[allow(clippy::too_many_arguments)]
pub async fn serve(host: String, port: String, config: AppConfig) {
    // Register the error channel before installing the tracing layer so errors
    // logged during boot are not dropped.
    register_error_channel();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_log_filter().into()),
        )
        .with(ErrorChannelLayer::default())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = async_db(&config.vec_db_path)
        .await
        .expect("Failed to connect to async db");

    let skill_registry = SkillRegistry::new(&config.skills_path)
        .await
        .expect("Failed to create skill registry");
    let app_state = AppState::new(db.clone(), config.clone(), skill_registry);
    let shared_state = Arc::new(RwLock::new(app_state));
    let app = app(Arc::clone(&shared_state));

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port))
        .await
        .unwrap();

    tracing::debug!(
        "Server started. Listening on {}",
        listener.local_addr().unwrap()
    );

    // Start default timer channels before respawning loops: their publishers
    // must be registered before loops subscribe to them. Failures are logged
    // and skipped.
    if let Err(e) = start_default_channels(&db).await {
        tracing::warn!("failed to start default channels: {}", e);
    }

    // Agent-created server channels lose their in-memory publisher on restart;
    // re-register them so loops subscribed to them can respawn below.
    if let Err(e) = register_agent_channels(&db).await {
        tracing::warn!("failed to register agent channels: {}", e);
    }

    // Respawn persisted channel loops before the periodic jobs (which move
    // `config`). Failures are logged and skipped so a stale DB or a down
    // publisher never blocks boot.
    let loop_config = LoopSupervisorConfig {
        db: db.clone(),
        api_base_url: config.note_search_api_url.clone(),
        api_hostname: config.openai_api_hostname.clone(),
        api_key: config.openai_api_key.clone(),
        model: config.openai_model.clone(),
        storage_path: config.storage_path.clone(),
        vapid_key_path: config.vapid_key_path.clone(),
    };
    if let Err(e) = loops::respawn_all(loop_config).await {
        tracing::warn!("failed to respawn channel loops: {}", e);
    }

    // Run background jobs. Each job is spawned in it's own tokio task
    // in a loop.
    spawn_periodic_job(config.clone(), db.clone(), DailyAgenda);
    spawn_periodic_job(config.clone(), db.clone(), GitSync);
    spawn_periodic_job(config, db, GenerateSessionTitles);

    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    use super::default_log_filter;

    /// In-memory writer for capturing formatted log output.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// The default filter must log `ERROR` events from targets it has no
    /// explicit directive for, while leaving their lower levels off.
    #[test]
    fn default_filter_logs_errors_from_unlisted_targets() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(default_log_filter()))
            .with(tracing_subscriber::fmt::layer().with_writer(buffer.clone()));

        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "unlisted_crate", "debug noise");
            tracing::error!(target: "unlisted_crate", "boom");
        });

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("boom"), "{output}");
        assert!(!output.contains("debug noise"), "{output}");
    }
}
