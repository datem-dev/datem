use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use datem_core::db::tables::subscriptions::Subscription;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppState, error::err};

// ── Request DTOs ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateSubscriptionReq {
    id: Option<String>,
    customer_id: Option<String>,
    plan_id: Option<String>,
    stripe_subscription_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSubscriptionReq {
    plan_id: Option<String>,
    effective: Option<String>,
}

// ── Response helper ───────────────────────────────────────────────────────────

fn sub_to_json(s: &Subscription) -> Value {
    json!({
        "id":                     s.id,
        "customer_id":            s.customer_id,
        "plan_id":                s.plan_id,
        "status":                 s.status,
        "current_period_start":   s.current_period_start,
        "current_period_end":     s.current_period_end,
        "stripe_subscription_id": s.stripe_subscription_id,
        "created_at":             s.created_at,
        "cancelled_at":           if s.cancelled_at == 0 { json!(null) } else { json!(s.cancelled_at) },
    })
}

// ── Stripe helper ─────────────────────────────────────────────────────────────

struct StripeSubResult {
    id: String,
    period_start_us: i64,
    period_end_us: i64,
}

async fn stripe_create_subscription(
    stripe_key: &str,
    stripe_customer_id: &str,
    stripe_interval: &str,
    http: &reqwest::Client,
) -> Result<StripeSubResult, Response> {
    let resp = http
        .post("https://api.stripe.com/v1/subscriptions")
        .bearer_auth(stripe_key)
        .form(&[
            ("customer", stripe_customer_id),
            ("items[0][price_data][currency]", "usd"),
            ("items[0][price_data][recurring][interval]", stripe_interval),
            ("items[0][price_data][product_data][name]", "Datem Subscription"),
            ("items[0][price_data][unit_amount]", "0"),
        ])
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(err(StatusCode::BAD_GATEWAY, "stripe_error", text, None));
    }

    let data: Value = resp.json().await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, "stripe_error", e.to_string(), None))?;

    let id = data["id"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "stripe_error", "missing id in Stripe response", None))?
        .to_string();

    let period_start_us = data["current_period_start"].as_i64().unwrap_or(0) * 1_000_000;
    let period_end_us = data["current_period_end"].as_i64().unwrap_or(0) * 1_000_000;

    Ok(StripeSubResult { id, period_start_us, period_end_us })
}

// ── Billing period helpers ────────────────────────────────────────────────────

fn billing_period(interval: &str) -> (i64, i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let duration_us = match interval {
        "annual" => 365i64 * 24 * 3600 * 1_000_000,
        _ => 30i64 * 24 * 3600 * 1_000_000,
    };
    (now, now + duration_us)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateSubscriptionReq>,
) -> Response {
    let id = match body.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "id is required", Some("id")),
    };
    let customer_id = match body.customer_id.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "customer_id is required", Some("customer_id")),
    };
    let plan_id = match body.plan_id.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => p.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "plan_id is required", Some("plan_id")),
    };

    // Validate customer and plan exist.
    match state.db.get_customer(&customer_id).await {
        Ok(None) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_found",
            format!("customer '{customer_id}' not found"), Some("customer_id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(Some(_)) => {}
    }
    let plan = match state.db.get_plan(&plan_id).await {
        Ok(None) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_found",
            format!("plan '{plan_id}' not found"), Some("plan_id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(Some(p)) => p,
    };
    if plan.status == "archived" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "plan_archived",
            format!("plan '{plan_id}' is archived; new subscriptions require an active plan"), Some("plan_id"));
    }

    match state.db.subscription_exists(&id).await {
        Ok(true) => return err(StatusCode::CONFLICT, "already_exists",
            format!("subscription '{id}' already exists"), Some("id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(false) => {}
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    // Resolve Stripe subscription ID and billing period.
    let (stripe_sub_id, period_start, period_end) =
        if let Some(sid) = body.stripe_subscription_id.filter(|s| !s.is_empty()) {
            let (start, end) = billing_period(&plan.interval);
            (sid, start, end)
        } else if let Some(sk) = state.config.stripe_key.as_deref().filter(|k| !k.contains("placeholder")) {
            // Need the customer's stripe_customer_id.
            let customer = match state.db.get_customer(&customer_id).await {
                Ok(Some(c)) => c,
                _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", "failed to reload customer", None),
            };
            if customer.stripe_customer_id.is_empty() {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "no_stripe_customer",
                    "customer has no Stripe customer ID — create the customer without a stripe_customer_id to auto-create one",
                    Some("customer_id"));
            }
            let stripe_interval = if plan.interval == "annual" { "year" } else { "month" };
            match stripe_create_subscription(sk, &customer.stripe_customer_id, stripe_interval, &state.http).await {
                Ok(r) => (r.id, r.period_start_us, r.period_end_us),
                Err(resp) => return resp,
            }
        } else {
            let (start, end) = billing_period(&plan.interval);
            (String::new(), start, end)
        };

    let sub = Subscription {
        id: id.clone(),
        tenant_id: "default".to_string(),
        customer_id,
        plan_id,
        status: "active".to_string(),
        current_period_start: period_start,
        current_period_end: period_end,
        stripe_subscription_id: stripe_sub_id,
        created_at: now,
        cancelled_at: 0,
    };

    if let Err(e) = state.db.insert_subscription(sub.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }

    (StatusCode::CREATED, Json(sub_to_json(&sub))).into_response()
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.db.get_subscription(&id).await {
        Ok(Some(s)) => Json(sub_to_json(&s)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found", format!("subscription '{id}' not found"), None),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    }
}

pub async fn list_for_customer(
    State(state): State<AppState>,
    Path(customer_id): Path<String>,
) -> Response {
    match state.db.get_customer(&customer_id).await {
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("customer '{customer_id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(Some(_)) => {}
    }

    match state.db.list_subscriptions_for_customer(&customer_id).await {
        Ok(subs) => {
            let data: Vec<Value> = subs.iter().map(sub_to_json).collect();
            Json(json!({ "data": data })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSubscriptionReq>,
) -> Response {
    let mut sub = match state.db.get_subscription(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("subscription '{id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    if sub.status == "cancelled" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "subscription_cancelled",
            "cannot update a cancelled subscription", None);
    }

    let new_plan_id = match body.plan_id.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => p.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "plan_id is required", Some("plan_id")),
    };

    match state.db.get_plan(&new_plan_id).await {
        Ok(None) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_found",
            format!("plan '{new_plan_id}' not found"), Some("plan_id")),
        Ok(Some(p)) if p.status == "archived" => return err(StatusCode::UNPROCESSABLE_ENTITY, "plan_archived",
            format!("plan '{new_plan_id}' is archived"), Some("plan_id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(Some(_)) => {}
    }

    // effective is informational for now — migration takes effect at next period in the billing engine.
    let _effective = body.effective.as_deref().unwrap_or("next_period");

    sub.plan_id = new_plan_id;

    if let Err(e) = state.db.insert_subscription(sub.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }

    Json(sub_to_json(&sub)).into_response()
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let mut sub = match state.db.get_subscription(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("subscription '{id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    if sub.status == "cancelled" {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "already_cancelled",
            "subscription is already cancelled", None);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let effective = params.get("effective").map(|s| s.as_str()).unwrap_or("period_end");
    sub.cancelled_at = match effective {
        "immediately" => now,
        _ => sub.current_period_end, // period_end: bill through to end of period
    };
    sub.status = "cancelled".to_string();

    if let Err(e) = state.db.insert_subscription(sub.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }

    Json(json!({
        "id":           sub.id,
        "status":       sub.status,
        "cancelled_at": sub.cancelled_at,
    })).into_response()
}
