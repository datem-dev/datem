use tonbo::prelude::*;

#[derive(Record, Debug)]
pub struct InvoiceLineItem {
    #[metadata(k = "tonbo.key", v = "true")]
    pub id: String,
    pub tenant_id: String,
    pub invoice_id: String,
    pub metric: String,
    pub description: String,
    pub quantity: f64,
    pub amount_cents: i64,
    pub model: String,
}
