use tonbo::prelude::*;

/// `completed_at` is 0 while the run is pending or failed.
/// `invoice_id` is empty string until the run completes successfully.
#[derive(Record, Debug)]
pub struct BillingRun {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub subscription_id: String,
    pub plan_id: String,
    pub period_start: i64,
    pub period_end: i64,
    pub status: String,
    pub invoice_id: String,
    pub created_at: i64,
    pub completed_at: i64,
}
