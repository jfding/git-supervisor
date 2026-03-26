use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::sync::Arc;
use tokio::sync::mpsc;

type HmacSha256 = Hmac<Sha256>;

struct HookState {
    secret: String,
    version: String,
    tx: mpsc::Sender<()>,
}

pub(crate) fn verify_signature(secret: &str, payload: &[u8], signature_header: &str) -> bool {
    let Some(hex_sig) = signature_header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(payload);
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    mac.verify_slice(&expected).is_ok()
}

async fn version_handler(State(state): State<Arc<HookState>>) -> impl IntoResponse {
    Json(json!({ "version": state.version }))
}

async fn webhook_handler(
    State(state): State<Arc<HookState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_signature(&state.secret, &body, signature) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Invalid signature" })),
        );
    }

    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event != "push" {
        return (
            StatusCode::OK,
            Json(json!({ "status": "Event not handled" })),
        );
    }

    // try_send: if channel is full (capacity 1), a cycle is already pending — drop this event
    match state.tx.try_send(()) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "status": "CI job triggered", "version": state.version })),
        ),
        Err(mpsc::error::TrySendError::Full(_)) => (
            StatusCode::OK,
            Json(json!({ "status": "Cycle already in progress, event dropped", "version": state.version })),
        ),
        Err(mpsc::error::TrySendError::Closed(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Watch loop has stopped" })),
        ),
    }
}

pub(crate) fn build_webhook_router(
    secret: String,
    version: String,
    tx: mpsc::Sender<()>,
) -> Router {
    let state = Arc::new(HookState {
        secret,
        version,
        tx,
    });

    Router::new()
        .route("/version", get(version_handler))
        .route("/webhook", post(webhook_handler))
        .with_state(state)
}

/// Start the webhook HTTP server in the background.
/// Returns the mpsc::Receiver that the event loop reads from.
pub(crate) async fn start_webhook_server(
    port: u16,
    secret: String,
    version: String,
) -> Result<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel(1);
    let app = build_webhook_router(secret, version, tx);

    let addr = format!("0.0.0.0:{}", port);
    eprintln!("Webhook server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context("failed to bind address")?;

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("Webhook server error: {}", e);
        }
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn make_signature(secret: &str, payload: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn verify_valid_signature() {
        let secret = "test-secret";
        let payload = b"hello world";
        let sig = make_signature(secret, payload);
        assert!(verify_signature(secret, payload, &sig));
    }

    #[test]
    fn reject_invalid_signature() {
        assert!(!verify_signature("secret", b"payload", "sha256=0000"));
    }

    #[test]
    fn reject_missing_prefix() {
        assert!(!verify_signature("secret", b"payload", "bad-header"));
    }

    #[tokio::test]
    async fn valid_push_sends_signal() {
        let (tx, mut rx) = mpsc::channel(1);
        let app = build_webhook_router("secret".into(), "1.0.0".into(), tx);
        let body = b"{}";
        let sig = make_signature("secret", body);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Hub-Signature-256", sig)
            .header("X-GitHub-Event", "push")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "CI job triggered");

        // Signal should be in the channel
        assert!(rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn invalid_signature_returns_403_no_signal() {
        let (tx, mut rx) = mpsc::channel(1);
        let app = build_webhook_router("secret".into(), "1.0.0".into(), tx);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Hub-Signature-256", "sha256=bad")
            .header("X-GitHub-Event", "push")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn non_push_event_does_not_signal() {
        let (tx, mut rx) = mpsc::channel(1);
        let app = build_webhook_router("secret".into(), "1.0.0".into(), tx);
        let body = b"{}";
        let sig = make_signature("secret", body);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("X-Hub-Signature-256", sig)
            .header("X-GitHub-Event", "issues")
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "Event not handled");

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn concurrent_webhook_dropped_when_channel_full() {
        let (tx, mut rx) = mpsc::channel(1);
        let app = build_webhook_router("secret".into(), "1.0.0".into(), tx);
        let body = b"{}";

        let build_req = || {
            Request::builder()
                .method("POST")
                .uri("/webhook")
                .header("X-Hub-Signature-256", make_signature("secret", body))
                .header("X-GitHub-Event", "push")
                .body(Body::from(&body[..]))
                .unwrap()
        };

        // First request fills the channel
        let resp = app.clone().oneshot(build_req()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Don't consume from rx — channel is now full

        // Second request should be dropped
        let resp = app.oneshot(build_req()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["status"].as_str().unwrap().contains("dropped"));

        // Only one signal in channel
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn version_endpoint_returns_version() {
        let (tx, _rx) = mpsc::channel(1);
        let app = build_webhook_router("secret".into(), "2.0.8".into(), tx);

        let req = Request::builder()
            .method("GET")
            .uri("/version")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], "2.0.8");
    }
}
