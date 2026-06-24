use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use datem_core::db::tables::metrics::Metric;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AppState, error::err};

const VALID_AGGREGATIONS: &[&str] = &["sum", "count", "max", "unique_count"];

#[derive(Deserialize)]
pub struct CreateMetricReq {
    pub id: Option<String>,
    pub display: Option<String>,
    pub aggregation: Option<String>,
}

#[derive(Serialize)]
pub struct MetricResponse {
    pub id: String,
    pub display: String,
    pub aggregation: String,
    pub created_at: i64,
}

impl From<&Metric> for MetricResponse {
    fn from(m: &Metric) -> Self {
        Self {
            id: m.id.clone(),
            display: m.display.clone(),
            aggregation: m.aggregation.clone(),
            created_at: m.created_at,
        }
    }
}

pub async fn create(State(state): State<AppState>, Json(req): Json<CreateMetricReq>) -> Response {
    let id = match req.id {
        Some(v) if !v.is_empty() => v,
        _ => return err(StatusCode::BAD_REQUEST, "missing_param", "id is required", Some("id")),
    };
    let display = match req.display {
        Some(v) if !v.is_empty() => v,
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                "missing_param",
                "display is required",
                Some("display"),
            )
        }
    };
    let aggregation = match req.aggregation {
        Some(v) if VALID_AGGREGATIONS.contains(&v.as_str()) => v,
        Some(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_param",
                format!("aggregation must be one of: {}", VALID_AGGREGATIONS.join(", ")),
                Some("aggregation"),
            )
        }
        None => {
            return err(
                StatusCode::BAD_REQUEST,
                "missing_param",
                "aggregation is required",
                Some("aggregation"),
            )
        }
    };

    match state.db.metric_exists(&id).await {
        Ok(true) => {
            return err(
                StatusCode::CONFLICT,
                "duplicate_id",
                format!("metric '{id}' already exists"),
                Some("id"),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to check metric existence");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to check metric existence",
                None,
            );
        }
        Ok(false) => {}
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let metric = Metric {
        id: id.clone(),
        tenant_id: "default".to_string(),
        display: display.clone(),
        aggregation: aggregation.clone(),
        status: "active".to_string(),
        created_at: now,
    };

    if let Err(e) = state.db.insert_metric(metric).await {
        tracing::error!(error = %e, "failed to insert metric");
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "failed to create metric",
            None,
        );
    }

    (
        StatusCode::CREATED,
        Json(json!(MetricResponse { id, display, aggregation, created_at: now })),
    )
        .into_response()
}

pub async fn list(State(state): State<AppState>) -> Response {
    match state.db.list_metrics().await {
        Ok(metrics) => {
            let data: Vec<MetricResponse> = metrics.iter().map(MetricResponse::from).collect();
            (StatusCode::OK, Json(json!({ "data": data }))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list metrics");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to list metrics",
                None,
            )
        }
    }
}

pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_metric(&id).await {
        Ok(Some(m)) => (StatusCode::OK, Json(json!(MetricResponse::from(&m)))).into_response(),
        Ok(None) => err(
            StatusCode::NOT_FOUND,
            "metric_not_found",
            format!("metric '{id}' not found"),
            None,
        ),
        Err(e) => {
            tracing::error!(error = %e, metric_id = %id, "failed to get metric");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to get metric",
                None,
            )
        }
    }
}

pub async fn archive(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.archive_metric(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(
            StatusCode::NOT_FOUND,
            "metric_not_found",
            format!("metric '{id}' not found"),
            None,
        ),
        Err(e) => {
            tracing::error!(error = %e, metric_id = %id, "failed to archive metric");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to archive metric",
                None,
            )
        }
    }
}
