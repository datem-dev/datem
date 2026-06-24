use anyhow::Result;
use aws_lambda_events::event::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent};
use datem_core::db::DbHandle;
use datem_core::db::tables::events::Event;
use lambda_runtime::LambdaEvent;
use serde::Deserialize;

#[derive(Deserialize)]
struct IngestRecord {
    event_id: Option<String>,
    customer_id: Option<String>,
    metric: Option<String>,
    quantity: Option<f64>,
    timestamp: Option<i64>,
    #[serde(default)]
    properties: serde_json::Value,
}

pub async fn handler(
    event: LambdaEvent<SqsEvent>,
    db: &DbHandle,
) -> Result<SqsBatchResponse> {
    let mut batch_item_failures = Vec::new();
    let mut to_ingest: Vec<Event> = Vec::new();

    let now_us = now_micros();

    for record in &event.payload.records {
        let message_id = record.message_id.clone().unwrap_or_default();
        let body = match &record.body {
            Some(b) => b,
            None => {
                tracing::warn!(message_id, "sqs record missing body");
                batch_item_failures.push(sqs_failure(message_id));
                continue;
            }
        };

        let parsed: IngestRecord = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(message_id, error = %e, "failed to parse sqs record body");
                batch_item_failures.push(sqs_failure(message_id));
                continue;
            }
        };

        match validate(parsed, now_us) {
            Ok(event) => to_ingest.push(event),
            Err(e) => {
                // Validation failures are permanent — don't return as SQS failures
                // so they aren't retried endlessly into the DLQ.
                tracing::warn!(message_id, error = %e, "invalid event, skipping");
            }
        }
    }

    if !to_ingest.is_empty() {
        let n = to_ingest.len();
        if let Err(e) = db.ingest_events(to_ingest).await {
            tracing::error!(error = %e, "db ingest_events failed — marking all records for retry");
            return Ok(SqsBatchResponse {
                batch_item_failures: event.payload.records.iter()
                    .filter_map(|r| r.message_id.clone())
                    .map(sqs_failure)
                    .collect(),
            });
        }
        tracing::info!(count = n, "ingested events from sqs batch");
    }

    Ok(SqsBatchResponse { batch_item_failures })
}

fn validate(r: IngestRecord, now_us: i64) -> Result<Event> {
    let event_id = r.event_id.filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("event_id is required"))?;
    let customer_id = r.customer_id.filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("customer_id is required"))?;
    let metric = r.metric.filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("metric is required"))?;
    let quantity = r.quantity
        .ok_or_else(|| anyhow::anyhow!("quantity is required"))?;
    let timestamp = r.timestamp
        .ok_or_else(|| anyhow::anyhow!("timestamp is required"))?;

    const MAX_BACKDATE_US: i64 = 24 * 3600 * 1_000_000;
    if timestamp > now_us + 60 * 1_000_000 {
        anyhow::bail!("timestamp is more than 60 seconds in the future");
    }
    if timestamp < now_us - MAX_BACKDATE_US {
        anyhow::bail!("timestamp is more than 24 hours in the past");
    }

    let properties = serde_json::to_string(&r.properties).unwrap_or_else(|_| "{}".to_string());

    Ok(Event {
        id: event_id,
        tenant_id: "default".to_string(),
        customer_id,
        metric,
        quantity,
        timestamp,
        properties,
    })
}

fn sqs_failure(message_id: String) -> BatchItemFailure {
    BatchItemFailure { item_identifier: message_id }
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}
