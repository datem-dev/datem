use tonbo::prelude::*;

#[derive(Record, Debug, Clone)]
pub struct Plan {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub status: String,
    pub currency: String,
    pub interval: String,
    pub created_at: i64,
}
