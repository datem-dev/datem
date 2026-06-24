use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use datem_core::db::tables::customers::Customer;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppState, error::err};

// ── Request DTOs ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateCustomerReq {
    id: Option<String>,
    name: Option<String>,
    email: Option<String>,
    stripe_customer_id: Option<String>,
    metadata: Option<Value>,
}

#[derive(Deserialize)]
pub struct UpdateCustomerReq {
    name: Option<String>,
    email: Option<String>,
    metadata: Option<Value>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<usize>,
    after: Option<String>,
}

// ── Response helper ───────────────────────────────────────────────────────────

fn customer_to_json(c: &Customer) -> Value {
    let metadata: Value = serde_json::from_str(&c.metadata).unwrap_or(json!({}));
    json!({
        "id":                 c.id,
        "name":               c.name,
        "email":              c.email,
        "stripe_customer_id": c.stripe_customer_id,
        "metadata":           metadata,
        "created_at":         c.created_at,
    })
}

// ── Stripe helpers ────────────────────────────────────────────────────────────

async fn stripe_create_customer(
    stripe_key: &str,
    name: &str,
    email: &str,
    http: &reqwest::Client,
) -> Result<String, Response> {
    let resp = http
        .post("https://api.stripe.com/v1/customers")
        .bearer_auth(stripe_key)
        .form(&[("name", name), ("email", email)])
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(err(StatusCode::BAD_GATEWAY, "stripe_error", text, None));
    }

    let data: Value = resp.json().await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    data["id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "stripe_error", "missing id in Stripe response", None))
        .map(|s| s.to_string())
}

async fn stripe_portal_session(
    stripe_key: &str,
    stripe_customer_id: &str,
    return_url: &str,
    http: &reqwest::Client,
) -> Result<String, Response> {
    let resp = http
        .post("https://api.stripe.com/v1/billing_portal/sessions")
        .bearer_auth(stripe_key)
        .form(&[("customer", stripe_customer_id), ("return_url", return_url)])
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(err(StatusCode::BAD_GATEWAY, "stripe_error", text, None));
    }

    let data: Value = resp.json().await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    data["url"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "stripe_error", "missing url in Stripe response", None))
        .map(|s| s.to_string())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateCustomerReq>,
) -> Response {
    let id = match body.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "id is required", Some("id")),
    };
    let name = match body.name.as_deref().filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "name is required", Some("name")),
    };
    let email = match body.email.as_deref().filter(|s| !s.is_empty()) {
        Some(e) => e.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "email is required", Some("email")),
    };

    match state.db.customer_exists(&id).await {
        Ok(true) => return err(StatusCode::CONFLICT, "already_exists",
            format!("customer '{id}' already exists"), Some("id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(false) => {}
    }

    // Resolve Stripe customer ID: use provided, or create one, or skip for dev.
    let stripe_customer_id = if let Some(sid) = body.stripe_customer_id.filter(|s| !s.is_empty()) {
        sid
    } else if let Some(sk) = state.config.stripe_key.as_deref().filter(|k| !k.contains("placeholder")) {
        match stripe_create_customer(sk, &name, &email, &state.http).await {
            Ok(sid) => sid,
            Err(r) => return r,
        }
    } else {
        String::new()
    };

    let metadata_str = body.metadata
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok())
        .unwrap_or_else(|| "{}".to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let customer = Customer {
        id: id.clone(),
        tenant_id: "default".to_string(),
        name,
        email,
        stripe_customer_id,
        metadata: metadata_str,
        created_at: now,
    };

    if let Err(e) = state.db.insert_customer(customer.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }

    (StatusCode::CREATED, Json(customer_to_json(&customer))).into_response()
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(20).min(100).max(1);
    let after = params.after.as_deref();

    match state.db.list_customers(limit, after).await {
        Ok((customers, has_more)) => {
            let next = if has_more { customers.last().map(|c| c.id.clone()) } else { None };
            let data: Vec<Value> = customers.iter().map(customer_to_json).collect();
            Json(json!({ "data": data, "has_more": has_more, "next": next })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    }
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.db.get_customer(&id).await {
        Ok(Some(c)) => Json(customer_to_json(&c)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found", format!("customer '{id}' not found"), None),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateCustomerReq>,
) -> Response {
    let mut customer = match state.db.get_customer(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("customer '{id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    if let Some(name) = body.name.filter(|s| !s.is_empty()) {
        customer.name = name;
    }
    if let Some(email) = body.email.filter(|s| !s.is_empty()) {
        customer.email = email;
    }
    if let Some(metadata) = body.metadata {
        customer.metadata = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
    }

    if let Err(e) = state.db.insert_customer(customer.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }

    Json(customer_to_json(&customer)).into_response()
}

pub async fn portal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let customer = match state.db.get_customer(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("customer '{id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    if customer.stripe_customer_id.is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "no_stripe_customer",
            "customer has no Stripe customer ID — create a subscription first", None);
    }

    let stripe_key = match state.config.stripe_key.as_deref().filter(|k| !k.contains("placeholder")) {
        Some(k) => k,
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "stripe_not_configured",
            "DATEM_STRIPE_KEY is not set", None),
    };

    let return_url = params.get("return_url").map(|s| s.as_str()).unwrap_or("https://example.com");

    let url = match stripe_portal_session(stripe_key, &customer.stripe_customer_id, return_url, &state.http).await {
        Ok(u) => u,
        Err(r) => return r,
    };

    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
        + 300 * 1_000_000; // 5 minutes in microseconds

    Json(json!({ "url": url, "expires_at": expires_at })).into_response()
}

pub async fn usage(
    Path(_id): Path<String>,
) -> Response {
    err(StatusCode::NOT_IMPLEMENTED, "not_implemented",
        "usage aggregation is implemented in step 10", None)
}
