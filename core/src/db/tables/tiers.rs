use tonbo::prelude::*;

/// `up_to` of -1 means the tier is unbounded (inf).
/// All monetary fields are in integer cents (minor currency units).
#[derive(Record, Debug, Clone)]
pub struct Tier {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub charge_id: String,
    pub up_to: i64,
    pub unit_price: i64,
    pub flat_fee: i64,
    pub position: i32,
}
