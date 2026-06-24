use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use super::AppState;

pub async fn bearer_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let token = extract_bearer(request.headers()).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or malformed Authorization header"})),
        )
            .into_response()
    })?;

    if token != state.config.api_key {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid API key"})),
        )
            .into_response());
    }

    Ok(next.run(request).await)
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
