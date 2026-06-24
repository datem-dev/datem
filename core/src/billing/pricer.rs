use crate::db::tables::{charges::Charge, tiers::Tier};

pub struct LineItem {
    pub metric: String,
    pub description: String,
    pub quantity: f64,
    pub amount_cents: i64,
    pub model: String,
}

/// Compute a line item for `charge` given `usage` units and the associated tiers (if any).
/// All monetary values are in integer minor currency units (cents).
pub fn price_charge(charge: &Charge, tiers: &[Tier], usage: f64) -> LineItem {
    let amount_cents = match charge.model.as_str() {
        "flat" => charge.flat_amount,

        "per_unit" => (usage * charge.unit_price as f64).round() as i64,

        "package" => {
            if charge.package_size <= 0 {
                0
            } else {
                let blocks = (usage / charge.package_size as f64).ceil() as i64;
                blocks * charge.unit_price
            }
        }

        "hybrid" => {
            charge.flat_amount + (usage * charge.unit_price as f64).round() as i64
        }

        "tiered" => compute_tiered(tiers, usage),

        _ => 0,
    };

    let description = build_description(charge, usage);

    LineItem {
        metric: charge.metric.clone(),
        description,
        quantity: usage,
        amount_cents,
        model: charge.model.clone(),
    }
}

/// Graduated tiered pricing: each tier's rate applies only to the units in that tier's range.
fn compute_tiered(tiers: &[Tier], usage: f64) -> i64 {
    let mut remaining = usage;
    let mut total_cents = 0i64;
    let mut prev_up_to = 0f64;

    for tier in tiers {
        if remaining <= 0.0 { break; }

        let tier_limit = if tier.up_to < 0 {
            f64::INFINITY
        } else {
            tier.up_to as f64
        };

        let tier_size = (tier_limit - prev_up_to).min(remaining);
        total_cents += (tier_size * tier.unit_price as f64).round() as i64;
        total_cents += tier.flat_fee;

        remaining -= tier_size;
        prev_up_to = tier_limit;
    }

    total_cents
}

fn build_description(charge: &Charge, usage: f64) -> String {
    if charge.display.is_empty() {
        match charge.model.as_str() {
            "flat" => "Flat fee".to_string(),
            "per_unit" => format!("{usage:.4} units × ${:.6}/unit", charge.unit_price as f64 / 100.0),
            "package" => format!("Package usage ({usage:.0} units)"),
            "hybrid" => format!("Base fee + {usage:.4} units"),
            "tiered" => format!("Tiered usage ({usage:.4} units)"),
            _ => format!("{} usage", charge.model),
        }
    } else {
        charge.display.clone()
    }
}
