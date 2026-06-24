use anyhow::{Context, Result};

use crate::stripe::client::StripeClient;
use super::pricer::LineItem;

/// Push line items to Stripe and create + finalize an invoice.
/// Returns the Stripe invoice ID.
pub async fn create_invoice(
    stripe: &StripeClient,
    stripe_customer_id: &str,
    currency: &str,
    line_items: &[LineItem],
) -> Result<String> {
    for item in line_items {
        if item.amount_cents == 0 { continue; }
        stripe
            .create_invoice_item(stripe_customer_id, item.amount_cents, currency, &item.description)
            .await
            .with_context(|| format!("create invoice item for metric '{}'", item.metric))?;
    }

    stripe
        .create_and_finalize_invoice(stripe_customer_id, currency)
        .await
        .context("create and finalize invoice")
}
