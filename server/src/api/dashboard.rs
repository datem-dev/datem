use axum::{http::{StatusCode, header}, response::IntoResponse};

const HTML: &str = include_str!("../dashboard.html");

pub async fn handler() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], HTML)
}
