use tonbo::prelude::*;

#[derive(Record, Debug, Clone)]
pub struct Customer {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub email: String,
    pub stripe_customer_id: String,
    pub metadata: String,
    pub created_at: i64,
}
