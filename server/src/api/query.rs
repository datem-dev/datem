use axum::{extract::State, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::json;

use super::{AppState, error::err};

#[derive(Deserialize)]
pub struct QueryReq {
    sql: Option<String>,
}

pub async fn query(
    State(state): State<AppState>,
    Json(body): Json<QueryReq>,
) -> Response {
    let sql = match body.sql.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "sql is required", Some("sql")),
    };

    // Basic safety: reject DDL and DML — SELECT/WITH/EXPLAIN/SHOW only
    let upper = sql.trim().to_uppercase();
    if !upper.starts_with("SELECT")
        && !upper.starts_with("WITH")
        && !upper.starts_with("EXPLAIN")
        && !upper.starts_with("SHOW")
    {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_query",
            "only SELECT, WITH, EXPLAIN, and SHOW queries are allowed",
            Some("sql"),
        );
    }

    match state.db.run_query(sql).await {
        Ok(result) => Json(json!({
            "columns": result.columns,
            "rows": result.rows,
            "row_count": result.row_count,
        })).into_response(),
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, "query_error", e.to_string(), Some("sql")),
    }
}
