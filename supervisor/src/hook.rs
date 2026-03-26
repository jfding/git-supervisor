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

use crate::CentralConfig;

type HmacSha256 = Hmac<Sha256>;

struct HookState {
    secret: String,
    config: CentralConfig,
    version: String,
}

fn verify_signature(secret: &str, payload: &[u8], signature_header: &str) -> bool {
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

    run_watch_handler(&state).await
}

async fn run_watch_handler(state: &Arc<HookState>) -> (StatusCode, Json<Value>) {
    let config = state.config.clone();
    let result =
        tokio::task::spawn_blocking(move || crate::run_watch(&config, 0, None, false, true)).await;

    match result {
        Ok(Ok(())) => {
            let mut payload = json!({ "status": "CI job started", "engine": "supervisor-watch" });
            payload["version"] = json!(state.version);
            (StatusCode::OK, Json(payload))
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Watch failed: {}", e) })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Task join error: {}", e) })),
        ),
    }
}

/// Start the webhook HTTP server.
///
/// Push events trigger a one-shot `run_watch` cycle using the config.
pub async fn run_hook(
    config: CentralConfig,
    port: u16,
    secret: String,
    version: String,
) -> Result<()> {
    let state = Arc::new(HookState {
        secret,
        config,
        version,
    });

    let app = Router::new()
        .route("/version", get(version_handler))
        .route("/webhook", post(webhook_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    eprintln!("Webhook server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context("failed to bind address")?;

    axum::serve(listener, app).await.context("server error")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_signature() {
        let secret = "test-secret";
        let payload = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
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
}
