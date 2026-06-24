use axum::{http::StatusCode, response::Json};
use serde_json::{Value, json};

pub async fn handle_stripe() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}
