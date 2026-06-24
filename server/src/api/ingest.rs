use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use datem_core::db::DbHandle;
use datem_core::db::tables::events::Event;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{AppState, IngestStats, error::err};

const MAX_BATCH: usize = 1000;
const MAX_BACKDATE_US: i64 = 24 * 3600 * 1_000_000; // 24 hours in microseconds

// ── Request DTOs ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct IngestOneReq {
    event_id: Option<String>,
    customer_id: Option<String>,
    metric: Option<String>,
    quantity: Option<f64>,
    timestamp: Option<i64>,
    properties: Option<Value>,
}

#[derive(Deserialize)]
pub struct IngestBatchReq {
    events: Option<Vec<IngestOneReq>>,
}

// ── Validation ────────────────────────────────────────────────────────────────

struct ValidatedEvent {
    event_id: String,
    customer_id: String,
    metric: String,
    quantity: f64,
    timestamp: i64,
    properties: String,
}

fn validate_fields(req: &IngestOneReq, now_us: i64) -> Result<ValidatedEvent, String> {
    let event_id = req.event_id.as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "event_id is required".to_string())?
        .to_string();

    let customer_id = req.customer_id.as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "customer_id is required".to_string())?
        .to_string();

    let metric = req.metric.as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "metric is required".to_string())?
        .to_string();

    let quantity = req.quantity
        .ok_or_else(|| "quantity is required".to_string())?;

    let timestamp = req.timestamp
        .ok_or_else(|| "timestamp is required".to_string())?;

    if timestamp > now_us + 60 * 1_000_000 {
        return Err(format!("timestamp {timestamp} is more than 60 seconds in the future"));
    }
    if timestamp < now_us - MAX_BACKDATE_US {
        return Err(format!("timestamp {timestamp} is more than 24 hours in the past"));
    }

    let properties = req.properties.as_ref()
        .and_then(|v| serde_json::to_string(v).ok())
        .unwrap_or_else(|| "{}".to_string());

    Ok(ValidatedEvent { event_id, customer_id, metric, quantity, timestamp, properties })
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn ingest_one(
    State(state): State<AppState>,
    Json(body): Json<IngestOneReq>,
) -> Response {
    let now_us = now_micros();

    let v = match validate_fields(&body, now_us) {
        Ok(v) => v,
        Err(msg) => return err(StatusCode::UNPROCESSABLE_ENTITY, "invalid_event", msg, None),
    };

    // Validate customer and metric exist (cache-first to avoid a scan per request).
    match customer_exists_cached(&state, &v.customer_id).await {
        Ok(false) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_found",
            format!("customer '{}' not found", v.customer_id), Some("customer_id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(true) => {}
    }
    match metric_exists_cached(&state, &v.metric).await {
        Ok(false) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_found",
            format!("metric '{}' not found", v.metric), Some("metric")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(true) => {}
    }

    let event = Event {
        id: v.event_id.clone(),
        tenant_id: "default".to_string(),
        customer_id: v.customer_id,
        metric: v.metric,
        quantity: v.quantity,
        timestamp: v.timestamp,
        properties: v.properties,
    };

    if state.ingest_tx.send(event).await.is_err() {
        return err(StatusCode::SERVICE_UNAVAILABLE, "queue_unavailable", "ingest queue is not available", None);
    }

    (StatusCode::ACCEPTED, Json(json!({ "event_id": v.event_id, "status": "accepted" }))).into_response()
}

pub async fn ingest_batch(
    State(state): State<AppState>,
    Json(body): Json<IngestBatchReq>,
) -> Response {
    let raw_events = match body.events {
        Some(v) if !v.is_empty() => v,
        _ => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
            "events array is required and must not be empty", Some("events")),
    };

    if raw_events.len() > MAX_BATCH {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "batch_too_large",
            format!("batch size {} exceeds maximum of {MAX_BATCH}", raw_events.len()), Some("events"));
    }

    let now_us = now_micros();

    // Field-validate each event; collect validated + per-event errors.
    let mut validated: Vec<ValidatedEvent> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();

    for req in &raw_events {
        match validate_fields(req, now_us) {
            Ok(v) => validated.push(v),
            Err(msg) => {
                let event_id = req.event_id.as_deref().unwrap_or("<missing>").to_string();
                errors.push(json!({ "event_id": event_id, "error": msg }));
            }
        }
    }

    // Validate unique customer_ids (cache-first, one actor call per cache miss).
    let unique_customers: HashSet<String> = validated.iter().map(|v| v.customer_id.clone()).collect();
    let mut valid_customers: HashMap<String, bool> = HashMap::new();
    for cid in &unique_customers {
        let exists = customer_exists_cached(&state, cid).await.unwrap_or(false);
        valid_customers.insert(cid.clone(), exists);
    }

    // Validate unique metric names (cache-first).
    let unique_metrics: HashSet<String> = validated.iter().map(|v| v.metric.clone()).collect();
    let mut valid_metrics: HashMap<String, bool> = HashMap::new();
    for m in &unique_metrics {
        let exists = metric_exists_cached(&state, m).await.unwrap_or(false);
        valid_metrics.insert(m.clone(), exists);
    }

    // Split validated events into accepted / rejected based on existence checks.
    let mut to_ingest: Vec<Event> = Vec::new();
    for v in validated {
        let cust_ok = valid_customers.get(&v.customer_id).copied().unwrap_or(false);
        let metric_ok = valid_metrics.get(&v.metric).copied().unwrap_or(false);

        if !cust_ok {
            errors.push(json!({
                "event_id": v.event_id,
                "error": format!("customer '{}' not found", v.customer_id),
            }));
        } else if !metric_ok {
            errors.push(json!({
                "event_id": v.event_id,
                "error": format!("metric '{}' not found", v.metric),
            }));
        } else {
            to_ingest.push(Event {
                id: v.event_id,
                tenant_id: "default".to_string(),
                customer_id: v.customer_id,
                metric: v.metric,
                quantity: v.quantity,
                timestamp: v.timestamp,
                properties: v.properties,
            });
        }
    }

    let mut rejected = errors.len();
    let mut queued = to_ingest.len();
    for event in to_ingest {
        if state.ingest_tx.send(event).await.is_err() {
            queued -= 1;
            rejected += 1;
        }
    }
    let accepted = queued;

    (StatusCode::ACCEPTED, Json(json!({
        "accepted": accepted,
        "rejected": rejected,
        "errors":   errors,
    }))).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

async fn customer_exists_cached(state: &AppState, id: &str) -> anyhow::Result<bool> {
    if state.cache.customers.read().unwrap().contains(id) {
        return Ok(true);
    }
    let exists = state.db.customer_exists(id).await?;
    if exists {
        state.cache.customers.write().unwrap().insert(id.to_string());
    }
    Ok(exists)
}

async fn metric_exists_cached(state: &AppState, id: &str) -> anyhow::Result<bool> {
    if state.cache.metrics.read().unwrap().contains(id) {
        return Ok(true);
    }
    let exists = state.db.metric_exists(id).await?;
    if exists {
        state.cache.metrics.write().unwrap().insert(id.to_string());
    }
    Ok(exists)
}

// ── Background flush loop ─────────────────────────────────────────────────────

pub async fn flush_loop(
    mut rx: mpsc::Receiver<Event>,
    db: DbHandle,
    stats: Arc<IngestStats>,
) {
    use tokio::time::{timeout_at, Instant, Duration};
    loop {
        let mut batch = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(deadline, rx.recv()).await {
                Ok(Some(event)) => batch.push(event),
                Ok(None) => {
                    // channel closed — flush remaining and exit
                    if !batch.is_empty() {
                        let n = batch.len() as u64;
                        if db.ingest_events(batch).await.is_ok() {
                            stats.total_events.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    return;
                }
                Err(_) => break, // 50ms window elapsed
            }
        }
        if !batch.is_empty() {
            let n = batch.len() as u64;
            match db.ingest_events(batch).await {
                Ok(()) => { stats.total_events.fetch_add(n, std::sync::atomic::Ordering::Relaxed); }
                Err(e) => { tracing::error!("ingest flush failed, {} events dropped: {e}", n); }
            }
        }
    }
}
