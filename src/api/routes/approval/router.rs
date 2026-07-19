//! Router for the approval API

use std::sync::{Arc, RwLock};

use axum::{
    Json,
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};

use super::public::{ApprovalRequest, ApprovalResponse};
use crate::ai::chat::ApprovalDecision;
use crate::api::public::ApiError;
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Resolve a pending tool-call approval request.
///
/// The chat task that issued the approval request (via
/// `ApprovalMiddleware`) is blocked waiting on a response from this
/// endpoint. On match, the registry wakes up that task with the
/// user's decision.
///
/// Returns 200 regardless of whether a pending request was found —
/// callers can distinguish via the `resolved` field in the response.
/// This keeps double-clicks on Approve from surfacing as errors:
/// once a request is resolved, subsequent attempts are no-ops.
async fn resolve_approval(
    State(state): State<SharedState>,
    Path(session_id): Path<String>,
    Json(body): Json<ApprovalRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let registry = state
        .read()
        .expect("Unable to read shared state")
        .approval_registry
        .clone();

    let decision = if body.approved {
        ApprovalDecision::Approved
    } else {
        ApprovalDecision::Denied(body.message.unwrap_or_else(|| "User denied".to_string()))
    };

    let resolved = registry.resolve(&session_id, &body.request_id, decision);

    if !resolved {
        // No pending entry. Could be: already resolved (double-click),
        // timed out, or never existed. Either way we don't error —
        // returning 200 with `resolved: false` lets the client treat
        // this idempotently without showing an error for stale requests.
        return Ok((
            StatusCode::OK,
            Json(ApprovalResponse { resolved: false }),
        )
            .into_response());
    }

    Ok((
        StatusCode::OK,
        Json(ApprovalResponse { resolved: true }),
    )
        .into_response())
}

/// Create the approval router.
pub fn router() -> Router<SharedState> {
    Router::new().route("/{session_id}", post(resolve_approval))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::{ApprovalDecision, ApprovalRegistry};
    use axum::Router;
    use axum::body::{self, Body};
    use axum::http::Request;
    use std::time::Duration;
    use tower::ServiceExt;

    async fn test_state(registry: Arc<ApprovalRegistry>) -> SharedState {
        let db = tokio_rusqlite::Connection::open_in_memory()
            .await
            .expect("in-memory db");
        // Empty temp dir for the skill registry; we don't load any
        // skills in these tests.
        let temp = tempfile::TempDir::new().expect("temp dir");
        let skill_registry =
            crate::ai::skills::SkillRegistry::new(temp.path()).await.expect("skill registry");
        Arc::new(RwLock::new(AppState {
            latest_selection: None,
            db,
            config: crate::core::AppConfig::default(),
            skill_registry: std::sync::Arc::new(std::sync::RwLock::new(skill_registry)),
            approval_registry: registry,
        }))
    }

    #[tokio::test]
    async fn test_resolve_approval_happy_path() {
        let registry = Arc::new(ApprovalRegistry::new(Duration::from_secs(5)));
        let session_id = "s1";
        let request_id = "r1";

        // Spawn a waiter so the registry has a pending entry to resolve
        let r2 = registry.clone();
        let h = tokio::spawn(async move { r2.request(session_id, request_id).await });

        // Give the waiter a moment to register
        tokio::time::sleep(Duration::from_millis(10)).await;

        let state = test_state(registry.clone()).await;
        let app: Router = router().with_state(state);
        let body = serde_json::json!({
            "request_id": request_id,
            "approved": true,
        });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/{session_id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.expect("router responded");
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("body bytes");
        let parsed: ApprovalResponse =
            serde_json::from_slice(&bytes).expect("valid JSON");
        assert!(parsed.resolved);

        let decision = h.await.expect("wait task panicked");
        assert!(matches!(decision, ApprovalDecision::Approved));
    }

    #[tokio::test]
    async fn test_resolve_approval_no_pending_returns_resolved_false() {
        let registry = Arc::new(ApprovalRegistry::new(Duration::from_secs(1)));
        let state = test_state(registry).await;
        let app: Router = router().with_state(state);

        // No pending entry exists — endpoint should still return 200
        let body = serde_json::json!({
            "request_id": "nonexistent",
            "approved": true,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/s1")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.expect("router responded");
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("body bytes");
        let parsed: ApprovalResponse =
            serde_json::from_slice(&bytes).expect("valid JSON");
        assert!(!parsed.resolved);
    }

    #[tokio::test]
    async fn test_resolve_approval_denied_propagates_message() {
        let registry = Arc::new(ApprovalRegistry::new(Duration::from_secs(5)));
        let session_id = "s2";
        let request_id = "r2";

        let r2 = registry.clone();
        let h = tokio::spawn(async move { r2.request(session_id, request_id).await });
        tokio::time::sleep(Duration::from_millis(10)).await;

        let state = test_state(registry.clone()).await;
        let app: Router = router().with_state(state);

        let body = serde_json::json!({
            "request_id": request_id,
            "approved": false,
            "message": "not allowed",
        });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/{session_id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.expect("router responded");
        assert_eq!(resp.status(), StatusCode::OK);

        let decision = h.await.expect("wait task panicked");
        match decision {
            ApprovalDecision::Denied(msg) => assert_eq!(msg, "not allowed"),
            other => panic!("expected Denied with message, got {other:?}"),
        }
    }
}