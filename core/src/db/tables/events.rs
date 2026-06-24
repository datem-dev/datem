use tonbo::prelude::*;

#[derive(Record, Debug, Clone)]
pub struct Event {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub metric: String,
    pub quantity: f64,
    pub timestamp: i64,
    pub properties: String,
}
