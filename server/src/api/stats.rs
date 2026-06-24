use std::sync::atomic::Ordering;

use axum::{extract::State, response::{IntoResponse, Response}, Json};
use serde_json::json;

use super::AppState;

pub async fn handler(State(state): State<AppState>) -> Response {
    let total_events = state.ingest_stats.total_events.load(Ordering::Relaxed);
    let uptime_secs = state.ingest_stats.started_at.elapsed().as_secs_f64();
    let metrics_count = state.db.list_metrics().await.map(|v| v.len()).unwrap_or(0);
    let (customers, _) = state.db.list_customers(1000, None).await.unwrap_or_default();
    let plans = state.db.list_plans("active").await.unwrap_or_default();
    Json(json!({
        "events_total": total_events,
        "uptime_seconds": uptime_secs,
        "metrics": metrics_count,
        "customers": customers.len(),
        "plans": plans.len(),
    })).into_response()
}
