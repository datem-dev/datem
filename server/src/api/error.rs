use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

pub fn err(status: StatusCode, code: &str, message: impl Into<String>, param: Option<&str>) -> Response {
    let mut error = json!({
        "code": code,
        "message": message.into(),
    });
    if let Some(p) = param {
        error["param"] = json!(p);
    }
    (status, Json(json!({ "error": error }))).into_response()
}
