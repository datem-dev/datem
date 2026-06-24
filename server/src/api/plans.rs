use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use datem_core::db::tables::{charges::Charge, plans::Plan, tiers::Tier};
use serde::Deserialize;
use serde_json::{Value, json};
use ulid::Ulid;

use super::{AppState, error::err};

// ── Request DTOs ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreatePlanReq {
    id: Option<String>,
    name: Option<String>,
    currency: Option<String>,
    interval: Option<String>,
    #[serde(default)]
    charges: Vec<CreateChargeReq>,
}

#[derive(Deserialize)]
pub struct CreateChargeReq {
    metric: Option<String>,
    model: Option<String>,
    /// Per-unit or package price in integer cents.
    unit_price: Option<i64>,
    /// Flat charge amount and hybrid base fee in integer cents.
    amount: Option<i64>,
    package_size: Option<i64>,
    package_price: Option<i64>,
    display: Option<String>,
    #[serde(default)]
    tiers: Vec<CreateTierReq>,
}

#[derive(Deserialize)]
pub struct CreateTierReq {
    up_to: Option<i64>,
    /// Tier per-unit price in integer cents.
    unit_price: i64,
    /// Tier flat fee in integer cents.
    flat_fee: Option<i64>,
}

// ── Response helpers ──────────────────────────────────────────────────────────

fn plan_to_json(plan: &Plan, charges: &[Charge], tiers_by_charge: &HashMap<String, Vec<Tier>>) -> Value {
    let empty = vec![];
    let charges_json: Vec<Value> = charges
        .iter()
        .map(|c| charge_to_json(c, tiers_by_charge.get(&c.id).unwrap_or(&empty)))
        .collect();
    json!({
        "id":         plan.id,
        "name":       plan.name,
        "status":     plan.status,
        "currency":   plan.currency,
        "interval":   plan.interval,
        "charges":    charges_json,
        "created_at": plan.created_at,
    })
}

fn charge_to_json(charge: &Charge, tiers: &[Tier]) -> Value {
    match charge.model.as_str() {
        "flat" => {
            let mut obj = json!({
                "id":     charge.id,
                "model":  "flat",
                "metric": null,
                "amount": charge.flat_amount,
            });
            if !charge.display.is_empty() {
                obj["display"] = json!(charge.display);
            }
            obj
        }
        "per_unit" => json!({
            "id":         charge.id,
            "model":      "per_unit",
            "metric":     charge.metric,
            "unit_price": charge.unit_price,
        }),
        "tiered" => {
            let tiers_json: Vec<Value> = tiers
                .iter()
                .map(|t| {
                    let mut tier = json!({ "unit_price": t.unit_price });
                    tier["up_to"] = if t.up_to == -1 { json!(null) } else { json!(t.up_to) };
                    if t.flat_fee != 0 {
                        tier["flat_fee"] = json!(t.flat_fee);
                    }
                    tier
                })
                .collect();
            json!({
                "id":     charge.id,
                "model":  "tiered",
                "metric": charge.metric,
                "tiers":  tiers_json,
            })
        }
        "package" => json!({
            "id":            charge.id,
            "model":         "package",
            "metric":        charge.metric,
            "package_size":  charge.package_size,
            "package_price": charge.unit_price,
        }),
        "hybrid" => json!({
            "id":          charge.id,
            "model":       "hybrid",
            "metric":      charge.metric,
            "flat_amount": charge.flat_amount,
            "unit_price":  charge.unit_price,
        }),
        _ => json!({ "id": charge.id, "model": charge.model }),
    }
}

// ── Charge parsing ────────────────────────────────────────────────────────────

fn parse_charges(
    plan_id: &str,
    reqs: &[CreateChargeReq],
) -> Result<(Vec<Charge>, Vec<Tier>), Response> {
    let mut charges = Vec::new();
    let mut tiers = Vec::new();

    for (i, req) in reqs.iter().enumerate() {
        let model = req.model.as_deref().ok_or_else(|| {
            err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                format!("charges[{i}].model is required"), Some("charges"))
        })?;

        if !matches!(model, "flat" | "per_unit" | "tiered" | "package" | "hybrid") {
            return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "invalid_param",
                format!("charges[{i}].model '{model}' is invalid; must be flat, per_unit, tiered, package, or hybrid"),
                Some("charges")));
        }

        let charge_id = Ulid::new().to_string();
        let mut charge = Charge {
            id: charge_id.clone(),
            tenant_id: "default".to_string(),
            plan_id: plan_id.to_string(),
            metric: String::new(),
            model: model.to_string(),
            unit_price: 0,
            flat_amount: 0,
            package_size: 0,
            display: String::new(),
        };

        match model {
            "flat" => {
                charge.flat_amount = req.amount.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].amount is required for flat charges"), Some("charges"))
                })?;
                charge.display = req.display.clone().unwrap_or_default();
            }
            "per_unit" => {
                charge.metric = req.metric.clone().ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].metric is required for per_unit charges"), Some("charges"))
                })?;
                charge.unit_price = req.unit_price.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].unit_price is required for per_unit charges"), Some("charges"))
                })?;
            }
            "tiered" => {
                charge.metric = req.metric.clone().ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].metric is required for tiered charges"), Some("charges"))
                })?;
                if req.tiers.is_empty() {
                    return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].tiers must have at least one entry"), Some("charges")));
                }
                for (j, t) in req.tiers.iter().enumerate() {
                    tiers.push(Tier {
                        id: Ulid::new().to_string(),
                        tenant_id: "default".to_string(),
                        charge_id: charge_id.clone(),
                        up_to: t.up_to.unwrap_or(-1),
                        unit_price: t.unit_price,
                        flat_fee: t.flat_fee.unwrap_or(0),
                        position: j as i32,
                    });
                }
            }
            "package" => {
                charge.metric = req.metric.clone().ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].metric is required for package charges"), Some("charges"))
                })?;
                charge.package_size = req.package_size.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].package_size is required for package charges"), Some("charges"))
                })?;
                charge.unit_price = req.package_price.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].package_price is required for package charges"), Some("charges"))
                })?;
            }
            "hybrid" => {
                charge.metric = req.metric.clone().ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].metric is required for hybrid charges"), Some("charges"))
                })?;
                charge.flat_amount = req.amount.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].amount is required for hybrid charges"), Some("charges"))
                })?;
                charge.unit_price = req.unit_price.ok_or_else(|| {
                    err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
                        format!("charges[{i}].unit_price is required for hybrid charges"), Some("charges"))
                })?;
            }
            _ => unreachable!(),
        }

        charges.push(charge);
    }

    Ok((charges, tiers))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreatePlanReq>,
) -> Response {
    let id = match body.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "id is required", Some("id")),
    };
    let name = match body.name.as_deref().filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "name is required", Some("name")),
    };
    let currency = body.currency.as_deref().unwrap_or("usd").to_lowercase();
    let interval = body.interval.as_deref().unwrap_or("monthly");
    if !matches!(interval, "monthly" | "annual") {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "invalid_param",
            "interval must be monthly or annual", Some("interval"));
    }
    if body.charges.is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field",
            "at least one charge is required", Some("charges"));
    }

    let (charges, tiers) = match parse_charges(&id, &body.charges) {
        Ok(v) => v,
        Err(r) => return r,
    };

    match state.db.plan_exists(&id).await {
        Ok(true) => return err(StatusCode::CONFLICT, "already_exists",
            format!("plan '{id}' already exists"), Some("id")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        Ok(false) => {}
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let plan = Plan {
        id: id.clone(),
        tenant_id: "default".to_string(),
        name,
        status: "active".to_string(),
        currency,
        interval: interval.to_string(),
        created_at: now,
    };

    if let Err(e) = state.db.insert_plan(plan.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
    }
    for charge in &charges {
        if let Err(e) = state.db.insert_charge(charge.clone()).await {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
        }
    }
    for tier in &tiers {
        if let Err(e) = state.db.insert_tier(tier.clone()).await {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None);
        }
    }

    let mut tiers_by_charge: HashMap<String, Vec<Tier>> = HashMap::new();
    for tier in tiers {
        tiers_by_charge.entry(tier.charge_id.clone()).or_default().push(tier);
    }

    (StatusCode::CREATED, Json(plan_to_json(&plan, &charges, &tiers_by_charge))).into_response()
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let status = params.get("status").map(|s| s.as_str()).unwrap_or("active");
    if !matches!(status, "active" | "archived") {
        return err(StatusCode::BAD_REQUEST, "invalid_param",
            "status must be active or archived", Some("status"));
    }

    let plans = match state.db.list_plans(status).await {
        Ok(v) => v,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    let mut data = Vec::with_capacity(plans.len());
    for plan in &plans {
        let charges = match state.db.get_charges_for_plan(&plan.id).await {
            Ok(v) => v,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
        };
        let tiers_by_charge = match collect_tiers(&state, &charges).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        data.push(plan_to_json(plan, &charges, &tiers_by_charge));
    }

    Json(json!({ "data": data })).into_response()
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let plan = match state.db.get_plan(&id).await {
        Ok(Some(p)) => p,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", format!("plan '{id}' not found"), None),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };

    let charges = match state.db.get_charges_for_plan(&id).await {
        Ok(v) => v,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    };
    let tiers_by_charge = match collect_tiers(&state, &charges).await {
        Ok(v) => v,
        Err(r) => return r,
    };

    Json(plan_to_json(&plan, &charges, &tiers_by_charge)).into_response()
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.db.archive_plan(&id).await {
        Ok(true) => Json(json!({ "id": id, "status": "archived" })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "not_found",
            format!("plan '{id}' not found or already archived"), None),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None),
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

async fn collect_tiers(
    state: &AppState,
    charges: &[Charge],
) -> Result<HashMap<String, Vec<Tier>>, Response> {
    let mut map: HashMap<String, Vec<Tier>> = HashMap::new();
    for charge in charges {
        if charge.model == "tiered" {
            let tiers = state.db.get_tiers_for_charge(&charge.id).await.map_err(|e| {
                err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e.to_string(), None)
            })?;
            map.insert(charge.id.clone(), tiers);
        }
    }
    Ok(map)
}
