use anyhow::{Context, Result};

use crate::db::DbHandle;

pub struct MetricUsage {
    pub metric: String,
    pub quantity: f64,
}

/// Aggregate all usage events for a customer within [period_start, period_end) (microseconds).
pub async fn aggregate(
    db: &DbHandle,
    customer_id: &str,
    period_start: i64,
    period_end: i64,
) -> Result<Vec<MetricUsage>> {
    // Sanitize customer_id — it comes from our own DB so this is defensive only.
    if customer_id.contains('\'') {
        anyhow::bail!("invalid customer_id");
    }

    let sql = format!(
        "SELECT metric, SUM(quantity) AS total_quantity \
         FROM events \
         WHERE customer_id = '{customer_id}' \
           AND timestamp >= {period_start} \
           AND timestamp < {period_end} \
         GROUP BY metric"
    );

    let result = db.run_query(sql).await.context("aggregate usage query")?;

    let metric_col = result.columns.iter().position(|c| c == "metric")
        .context("missing metric column in aggregate result")?;
    let qty_col = result.columns.iter().position(|c| c == "total_quantity")
        .context("missing total_quantity column in aggregate result")?;

    let mut usages = Vec::with_capacity(result.row_count);
    for row in &result.rows {
        let metric = row[metric_col].as_str()
            .context("metric value is not a string")?
            .to_string();
        let quantity = match &row[qty_col] {
            serde_json::Value::Number(n) => n.as_f64().context("quantity not f64")?,
            serde_json::Value::Null => 0.0,
            other => anyhow::bail!("unexpected quantity type: {other}"),
        };
        usages.push(MetricUsage { metric, quantity });
    }
    Ok(usages)
}
