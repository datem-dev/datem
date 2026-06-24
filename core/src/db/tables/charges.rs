use tonbo::prelude::*;

/// `metric` is empty string for flat charges (no usage dimension).
/// `unit_price` and `flat_amount` are 0 for models that don't use them.
/// All monetary fields are in integer cents (minor currency units).
#[derive(Record, Debug, Clone)]
pub struct Charge {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub plan_id: String,
    pub metric: String,
    pub model: String,
    pub unit_price: i64,
    pub flat_amount: i64,
    pub package_size: i64,
    pub display: String,
}
