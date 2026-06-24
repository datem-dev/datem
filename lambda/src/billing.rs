use anyhow::Result;
use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use datem_core::billing::engine;
use datem_core::config::Config;
use datem_core::db::DbHandle;
use lambda_runtime::LambdaEvent;

pub async fn handler(
    _event: LambdaEvent<EventBridgeEvent<serde_json::Value>>,
    db: &DbHandle,
    config: &Config,
) -> Result<serde_json::Value> {
    tracing::info!("billing run started");

    let summary = engine::run(db, config).await?;

    if !summary.errors.is_empty() {
        for e in &summary.errors {
            tracing::error!(error = %e, "billing error");
        }
    }

    tracing::info!(
        processed = summary.subscriptions_processed,
        skipped = summary.subscriptions_skipped,
        errors = summary.errors.len(),
        "billing run complete"
    );

    Ok(serde_json::json!({
        "processed": summary.subscriptions_processed,
        "skipped":   summary.subscriptions_skipped,
        "errors":    summary.errors,
    }))
}
