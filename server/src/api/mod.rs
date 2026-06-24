pub mod auth;
pub mod error;
pub mod billing_runs;
pub mod customers;
pub mod dashboard;
pub mod ingest;
pub mod invoices;
pub mod metrics;
pub mod plans;
pub mod query;
pub mod router;
pub mod stats;
pub mod subscriptions;
pub mod webhooks;

use std::{
    collections::HashSet,
    sync::{
        Arc, RwLock,
        atomic::AtomicU64,
    },
};

use datem_core::{config::Config, db::DbHandle};
use datem_core::db::tables::events::Event;
use tokio::sync::mpsc;

/// Positive existence cache for customers and metrics.
///
/// Once we confirm a customer or metric exists in Tonbo we record it here so
/// subsequent ingest requests skip the full-table scan. Items are never removed
/// (customers/metrics are only soft-archived, never hard-deleted) so a cached
/// `true` never goes stale.
pub struct ExistenceCache {
    pub customers: RwLock<HashSet<String>>,
    pub metrics: RwLock<HashSet<String>>,
}

impl ExistenceCache {
    pub fn new() -> Self {
        Self {
            customers: RwLock::new(HashSet::new()),
            metrics: RwLock::new(HashSet::new()),
        }
    }
}

pub struct IngestStats {
    pub total_events: AtomicU64,
    pub started_at: std::time::Instant,
}

impl IngestStats {
    pub fn new() -> Self {
        Self {
            total_events: AtomicU64::new(0),
            started_at: std::time::Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    pub cache: Arc<ExistenceCache>,
    pub ingest_tx: mpsc::Sender<Event>,
    pub ingest_stats: Arc<IngestStats>,
}
