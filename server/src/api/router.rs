use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    Router,
    middleware,
    routing::{get, post, put},
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::info;

use datem_core::{config::Config, db::DbHandle};
use datem_core::db::tables::events::Event;
use super::{
    AppState, ExistenceCache, IngestStats, auth,
    billing_runs, customers, dashboard, ingest, invoices, metrics, plans, query, stats, subscriptions, webhooks,
};

pub async fn run(config: Config) -> Result<()> {
    let db = DbHandle::start(config.clone()).await?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let (ingest_tx, ingest_rx) = mpsc::channel::<Event>(8192);
    let ingest_stats = Arc::new(IngestStats::new());

    // Spawn background 50ms flush loop
    {
        let db = db.clone();
        let stats = ingest_stats.clone();
        tokio::spawn(super::ingest::flush_loop(ingest_rx, db, stats));
    }

    let state = AppState {
        db,
        config: Arc::new(config.clone()),
        http,
        cache: Arc::new(ExistenceCache::new()),
        ingest_tx,
        ingest_stats,
    };

    // All routes except webhooks and health require bearer token auth.
    let protected = Router::new()
        .route("/metrics", post(metrics::create).get(metrics::list))
        .route("/metrics/{id}", get(metrics::get_one).delete(metrics::archive))
        .route("/plans", post(plans::create).get(plans::list))
        .route("/plans/{id}", get(plans::get_one))
        .route("/plans/{id}/archive", put(plans::archive))
        .route("/customers", post(customers::create).get(customers::list))
        .route("/customers/{id}", get(customers::get_one).patch(customers::update))
        .route("/customers/{id}/usage", get(customers::usage))
        .route("/customers/{id}/portal", get(customers::portal))
        .route("/subscriptions", post(subscriptions::create))
        .route("/subscriptions/{id}", get(subscriptions::get_one).put(subscriptions::update).delete(subscriptions::cancel))
        .route("/customers/{id}/subscriptions", get(subscriptions::list_for_customer))
        .route("/ingest", post(ingest::ingest_one))
        .route("/ingest/batch", post(ingest::ingest_batch))
        .route("/query", post(query::query))
        .route("/billing-runs", get(billing_runs::list))
        .route("/billing-runs/trigger", post(billing_runs::trigger))
        .route("/invoices", get(invoices::list))
        .route("/invoices/{id}", get(invoices::get_one))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::bearer_auth,
        ));

    // Stripe webhooks use HMAC signature auth, not the bearer token.
    let app = Router::new()
        .merge(protected)
        .route("/webhooks/stripe", post(webhooks::handle_stripe))
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route("/stats", get(stats::handler))
        .route("/dashboard", get(dashboard::handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;
    info!(port = config.port, "datem listening");
    axum::serve(listener, app).await?;

    Ok(())
}
