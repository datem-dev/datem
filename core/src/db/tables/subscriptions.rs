use tonbo::prelude::*;

/// `cancelled_at` is 0 when the subscription is not cancelled.
#[derive(Record, Debug, Clone)]
pub struct Subscription {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub plan_id: String,
    pub status: String,
    pub current_period_start: i64,
    pub current_period_end: i64,
    pub stripe_subscription_id: String,
    pub created_at: i64,
    pub cancelled_at: i64,
}
