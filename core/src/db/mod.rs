pub mod actor;
pub mod store;
pub mod tables;

pub use actor::DbHandle;
pub use store::DatemStore;

/// Result of a SQL query executed via DataFusion over Tonbo data.
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
}
