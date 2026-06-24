use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

pub struct StripeClient {
    client: Client,
    key: String,
}

#[derive(Deserialize)]
struct StripeId {
    id: String,
}

impl StripeClient {
    pub fn new(key: impl Into<String>) -> Self {
        Self { client: Client::new(), key: key.into() }
    }

    pub async fn create_invoice_item(
        &self,
        stripe_customer_id: &str,
        amount_cents: i64,
        currency: &str,
        description: &str,
    ) -> Result<String> {
        let amount = amount_cents.to_string();
        let params = [
            ("customer", stripe_customer_id),
            ("amount", &amount),
            ("currency", currency),
            ("description", description),
        ];
        let res: StripeId = self.client
            .post("https://api.stripe.com/v1/invoiceitems")
            .basic_auth(&self.key, Option::<&str>::None)
            .form(&params)
            .send()
            .await
            .context("stripe create_invoice_item request")?
            .error_for_status()
            .context("stripe create_invoice_item status")?
            .json()
            .await
            .context("stripe create_invoice_item parse")?;
        Ok(res.id)
    }

    /// Creates an invoice for all pending invoice items on the customer and finalizes it.
    pub async fn create_and_finalize_invoice(
        &self,
        stripe_customer_id: &str,
        currency: &str,
    ) -> Result<String> {
        let invoice: StripeId = self.client
            .post("https://api.stripe.com/v1/invoices")
            .basic_auth(&self.key, Option::<&str>::None)
            .form(&[("customer", stripe_customer_id), ("currency", currency)])
            .send()
            .await
            .context("stripe create_invoice request")?
            .error_for_status()
            .context("stripe create_invoice status")?
            .json()
            .await
            .context("stripe create_invoice parse")?;

        let _: serde_json::Value = self.client
            .post(format!("https://api.stripe.com/v1/invoices/{}/finalize", invoice.id))
            .basic_auth(&self.key, Option::<&str>::None)
            .form(&[("auto_advance", "true")])
            .send()
            .await
            .context("stripe finalize_invoice request")?
            .error_for_status()
            .context("stripe finalize_invoice status")?
            .json()
            .await
            .context("stripe finalize_invoice parse")?;

        Ok(invoice.id)
    }
}
