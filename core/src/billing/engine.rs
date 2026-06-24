use anyhow::Result;
use ulid::Ulid;

use crate::config::Config;
use crate::db::DbHandle;
use crate::db::tables::{billing_runs::BillingRun, invoice_line_items::InvoiceLineItem, invoices::Invoice};
use crate::stripe::client::StripeClient;
use super::{aggregator, pricer, stripe as billing_stripe};

pub struct BillingRunSummary {
    pub subscriptions_processed: usize,
    pub subscriptions_skipped: usize,
    pub errors: Vec<String>,
}

/// Run billing for all active subscriptions whose current period has ended.
///
/// Idempotent — a billing run record is written before Stripe is called, so
/// a duplicate invocation for the same subscription+period is a no-op.
pub async fn run(db: &DbHandle, config: &Config) -> Result<BillingRunSummary> {
    let stripe_key = config.stripe_key.as_deref()
        .ok_or_else(|| anyhow::anyhow!("DATEM_STRIPE_KEY is required for billing"))?;
    let stripe = StripeClient::new(stripe_key);

    let now = now_micros();
    let subscriptions = db.list_active_subscriptions().await?;

    let mut processed = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for sub in subscriptions {
        if sub.current_period_end > now {
            skipped += 1;
            continue;
        }

        // Idempotency: skip if we already completed a run for this period.
        match db.billing_run_exists_for_period(&sub.id, sub.current_period_start).await {
            Ok(true) => { skipped += 1; continue; }
            Err(e) => { errors.push(format!("sub {}: check billing run: {e}", sub.id)); continue; }
            Ok(false) => {}
        }

        match bill_subscription(db, &stripe, &sub, now).await {
            Ok(()) => processed += 1,
            Err(e) => errors.push(format!("sub {}: {e}", sub.id)),
        }
    }

    Ok(BillingRunSummary { subscriptions_processed: processed, subscriptions_skipped: skipped, errors })
}

async fn bill_subscription(
    db: &DbHandle,
    stripe: &StripeClient,
    sub: &crate::db::tables::subscriptions::Subscription,
    now: i64,
) -> Result<()> {
    let run_id = Ulid::new().to_string();

    // Record the run as pending before doing anything external.
    let run = BillingRun {
        id: run_id.clone(),
        tenant_id: sub.tenant_id.clone(),
        customer_id: sub.customer_id.clone(),
        subscription_id: sub.id.clone(),
        plan_id: sub.plan_id.clone(),
        period_start: sub.current_period_start,
        period_end: sub.current_period_end,
        status: "pending".to_string(),
        invoice_id: String::new(),
        created_at: now,
        completed_at: 0,
    };
    db.insert_billing_run(run).await?;

    // Fetch plan, charges, and tiers.
    let plan = db.get_plan(&sub.plan_id).await?
        .ok_or_else(|| anyhow::anyhow!("plan '{}' not found", sub.plan_id))?;
    let charges = db.get_charges_for_plan(&sub.plan_id).await?;

    // Aggregate usage for the period.
    let usage = aggregator::aggregate(db, &sub.customer_id, sub.current_period_start, sub.current_period_end).await?;

    // Compute line items.
    let mut line_items = Vec::new();
    for charge in &charges {
        let tiers = db.get_tiers_for_charge(&charge.id).await?;
        let used = usage.iter()
            .find(|u| u.metric == charge.metric)
            .map(|u| u.quantity)
            .unwrap_or(0.0);
        line_items.push(pricer::price_charge(charge, &tiers, used));
    }

    let total_cents: i64 = line_items.iter().map(|l| l.amount_cents).sum();

    // Fetch the customer for the Stripe customer ID.
    let customer = db.get_customer(&sub.customer_id).await?
        .ok_or_else(|| anyhow::anyhow!("customer '{}' not found", sub.customer_id))?;

    // Push to Stripe.
    let stripe_invoice_id = billing_stripe::create_invoice(
        stripe,
        &customer.stripe_customer_id,
        &plan.currency,
        &line_items,
    ).await?;

    // Persist invoice and line items.
    let invoice_id = Ulid::new().to_string();
    db.insert_invoice(Invoice {
        id: invoice_id.clone(),
        tenant_id: sub.tenant_id.clone(),
        customer_id: sub.customer_id.clone(),
        subscription_id: sub.id.clone(),
        billing_run_id: run_id.clone(),
        stripe_invoice_id,
        status: "open".to_string(),
        currency: plan.currency.clone(),
        amount_cents: total_cents,
        period_start: sub.current_period_start,
        period_end: sub.current_period_end,
        created_at: now,
    }).await?;

    for item in &line_items {
        db.insert_invoice_line_item(InvoiceLineItem {
            id: Ulid::new().to_string(),
            tenant_id: sub.tenant_id.clone(),
            invoice_id: invoice_id.clone(),
            metric: item.metric.clone(),
            description: item.description.clone(),
            quantity: item.quantity,
            amount_cents: item.amount_cents,
            model: item.model.clone(),
        }).await?;
    }

    // Mark run as completed.
    db.insert_billing_run(BillingRun {
        id: Ulid::new().to_string(),
        tenant_id: sub.tenant_id.clone(),
        customer_id: sub.customer_id.clone(),
        subscription_id: sub.id.clone(),
        plan_id: sub.plan_id.clone(),
        period_start: sub.current_period_start,
        period_end: sub.current_period_end,
        status: "completed".to_string(),
        invoice_id: invoice_id.clone(),
        created_at: now,
        completed_at: now,
    }).await?;

    tracing::info!(
        subscription_id = %sub.id,
        customer_id = %sub.customer_id,
        invoice_id = %invoice_id,
        total_cents,
        "billing run completed"
    );

    Ok(())
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}
