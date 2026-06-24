use tonbo::prelude::*;

#[derive(Record, Debug)]
pub struct Metric {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub display: String,
    pub aggregation: String, // sum | count | max | unique_count
    pub status: String,      // active | archived
    pub created_at: i64,
}
