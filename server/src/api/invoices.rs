use axum::{extract::Path, http::StatusCode, response::Json};
use serde_json::{Value, json};

pub async fn list() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}

pub async fn get_one(Path(_id): Path<String>) -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_IMPLEMENTED, Json(json!({"error": "not implemented"})))
}
