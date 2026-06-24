use tonbo::prelude::*;

#[derive(Record, Debug)]
pub struct Invoice {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub subscription_id: String,
    pub billing_run_id: String,
    pub stripe_invoice_id: String,
    pub status: String,
    pub currency: String,
    pub amount_cents: i64,
    pub period_start: i64,
    pub period_end: i64,
    pub created_at: i64,
}
