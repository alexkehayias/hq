use std::sync::{Arc, RwLock};

use axum::middleware;
use axum::{Router, extract::Request, response::Response};
use http::{HeaderValue, header};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use super::routes;
use crate::ai::chat::{ChatSessionManager, ChatTaskDeps};
use crate::ai::pubsub::PubSubBroker;
use crate::ai::skills::SkillRegistry;
use crate::api::state::AppState;
use crate::core::{AppConfig, db::async_db};
use crate::jobs::{
    DailyAgenda, GenerateSessionTitles, ResearchMeetingAttendees, spawn_periodic_job,
};

async fn set_static_cache_control(request: Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

pub fn app(shared_state: Arc<RwLock<AppState>>) -> Router {
    let cors = CorsLayer::permissive();

    Router::new()
        // API routes
        .nest("/api", routes::router())
        // Static server of assets in ./web-ui
        .fallback_service(
            ServiceBuilder::new()
                .layer(middleware::from_fn(set_static_cache_control))
                .service(
                    ServeDir::new("./web-ui/src")
                        .precompressed_br()
                        .precompressed_gzip(),
                ),
        )
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(Arc::clone(&shared_state))
}

// Run the server
#[allow(clippy::too_many_arguments)]
pub async fn serve(host: String, port: String, config: AppConfig) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // axum logs rejections from built-in extractors with the `axum::rejection`
                // target, at `TRACE` level. `axum::rejection=trace` enables showing those events
                format! {
                    "{}=debug,tower_http=debug,axum::rejection=trace",
                    env!("CARGO_CRATE_NAME")
                }
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = async_db(&config.vec_db_path)
        .await
        .expect("Failed to connect to async db");

    let skill_registry = SkillRegistry::new(&config.skills_path)
        .await
        .expect("Failed to create skill registry");
    let skill_registry = Arc::new(RwLock::new(skill_registry));

    // Construct pub/sub broker and chat session manager. The broker is
    // shared across the process (handlers, jobs, tools can publish);
    // ChatSessionManager spawns a long-lived ChatTask per session that
    // owns the in-memory transcript.
    let broker = Arc::new(PubSubBroker::new());
    let chat_deps = ChatTaskDeps {
        db: db.clone(),
        config: Arc::new(config.clone()),
        skill_registry: Arc::clone(&skill_registry),
    };
    let chat_sessions = Arc::new(ChatSessionManager::new(
        Arc::clone(&broker),
        chat_deps,
    ));

    // Eagerly spawn ChatTasks for sessions with persisted subscriptions
    // so pub/sub messages aren't dropped before a session's first HTTP
    // request. Best-effort — log and continue if this fails.
    if let Err(e) = chat_sessions.restore_subscriptions().await {
        tracing::error!("Failed to restore chat subscriptions on startup: {}", e);
    }

    let app_state = AppState::new(
        db.clone(),
        config.clone(),
        // Pass the shared Arc<RwLock<SkillRegistry>> so AppState and
        // ChatTaskDeps see the same registry (skills loaded at startup).
        Arc::clone(&skill_registry),
        broker,
        chat_sessions,
    );
    let shared_state = Arc::new(RwLock::new(app_state));
    let app = app(Arc::clone(&shared_state));

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port))
        .await
        .unwrap();

    tracing::debug!(
        "Server started. Listening on {}",
        listener.local_addr().unwrap()
    );

    // Run background jobs. Each job is spawned in it's own tokio task
    // in a loop.
    spawn_periodic_job(config.clone(), db.clone(), DailyAgenda);
    spawn_periodic_job(config.clone(), db.clone(), ResearchMeetingAttendees);
    spawn_periodic_job(config, db, GenerateSessionTitles);

    axum::serve(listener, app).await.unwrap();
}
