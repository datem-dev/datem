use axum::{http::StatusCode, response::Json};
use serde_json::{Value, json};

pub async fn list() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

pub async fn trigger() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}
