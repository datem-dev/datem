use std::sync::Arc;

use anyhow::Result;
use fusio::{disk::LocalFs, executor::tokio::TokioExecutor};
use tonbo::db::{DB, DbBuildError, DbBuilder, Expr, NeverSeal, ScalarValue};
use tonbo::prelude::SchemaMeta;
use typed_arrow::AsViewsIterator;
use typed_arrow::prelude::{BuildRows, ViewResultIteratorExt};

use crate::config::Config;
use super::QueryResult;
use super::tables::{
    billing_runs::BillingRun,
    charges::Charge,
    customers::Customer,
    events::Event,
    invoice_line_items::InvoiceLineItem,
    invoices::Invoice,
    metrics::Metric,
    plans::Plan,
    subscriptions::Subscription,
    tiers::Tier,
};

type TonboDB = DB<LocalFs, TokioExecutor>;

pub struct DatemStore {
    pub events: TonboDB,
    pub customers: TonboDB,
    pub metrics: TonboDB,
    pub plans: TonboDB,
    pub charges: TonboDB,
    pub tiers: TonboDB,
    pub subscriptions: TonboDB,
    pub billing_runs: TonboDB,
    pub invoices: TonboDB,
    pub invoice_line_items: TonboDB,
}

impl DatemStore {
    pub async fn new(config: &Config) -> Result<Arc<Self>> {
        let data_dir_raw = std::path::Path::new(&config.data_dir);
        std::fs::create_dir_all(data_dir_raw)
            .map_err(|e| anyhow::anyhow!("failed to create data dir {:?}: {e}", data_dir_raw))?;
        let data_dir = data_dir_raw.canonicalize()
            .map_err(|e| anyhow::anyhow!("failed to resolve data dir {:?}: {e}", data_dir_raw))?;

        Ok(Arc::new(Self {
            events: open_table::<Event>(&data_dir, "events").await?,
            customers: open_table::<Customer>(&data_dir, "customers").await?,
            metrics: open_table::<Metric>(&data_dir, "metrics").await?,
            plans: open_table::<Plan>(&data_dir, "plans").await?,
            charges: open_table::<Charge>(&data_dir, "charges").await?,
            tiers: open_table::<Tier>(&data_dir, "tiers").await?,
            subscriptions: open_table::<Subscription>(&data_dir, "subscriptions").await?,
            billing_runs: open_table::<BillingRun>(&data_dir, "billing_runs").await?,
            invoices: open_table::<Invoice>(&data_dir, "invoices").await?,
            invoice_line_items: open_table::<InvoiceLineItem>(&data_dir, "invoice_line_items").await?,
        }))
    }

    // ── Metrics ──────────────────────────────────────────────────────────────

    pub async fn insert_metric(&self, metric: Metric) -> Result<()> {
        let mut builders = <Metric as BuildRows>::new_builders(1);
        builders.append_rows(vec![metric]);
        let batch = builders.finish().into_record_batch();
        self.metrics
            .ingest(batch)
            .await
            .map_err(|e| anyhow::anyhow!("ingest metric: {e}"))
    }

    pub async fn get_metric(&self, id: &str) -> Result<Option<Metric>> {
        let filter = Expr::and(vec![
            Expr::eq("id", ScalarValue::Utf8(Some(id.to_string()))),
            Expr::eq("status", ScalarValue::Utf8(Some("active".to_string()))),
        ]);
        let batches = self
            .metrics
            .scan()
            .filter(filter)
            .collect()
            .await
            .map_err(|e| anyhow::anyhow!("scan metric {id}: {e}"))?;

        for batch in &batches {
            let views = batch
                .iter_views::<Metric>()
                .map_err(|e| anyhow::anyhow!("metric schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("metric view access: {e}"))?;
            for view in views {
                return Ok(Some(Metric {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    display: view.display.to_string(),
                    aggregation: view.aggregation.to_string(),
                    status: view.status.to_string(),
                    created_at: view.created_at,
                }));
            }
        }
        Ok(None)
    }

    pub async fn metric_exists(&self, id: &str) -> Result<bool> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self
            .metrics
            .scan()
            .filter(filter)
            .collect()
            .await
            .map_err(|e| anyhow::anyhow!("scan metric {id}: {e}"))?;
        for batch in &batches {
            if batch.num_rows() > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn list_metrics(&self) -> Result<Vec<Metric>> {
        let filter = Expr::eq("status", ScalarValue::Utf8(Some("active".to_string())));
        let batches = self
            .metrics
            .scan()
            .filter(filter)
            .collect()
            .await
            .map_err(|e| anyhow::anyhow!("scan metrics: {e}"))?;

        let mut result = Vec::new();
        for batch in &batches {
            let views = batch
                .iter_views::<Metric>()
                .map_err(|e| anyhow::anyhow!("metric schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("metric view access: {e}"))?;
            for view in views {
                result.push(Metric {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    display: view.display.to_string(),
                    aggregation: view.aggregation.to_string(),
                    status: view.status.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    pub async fn archive_metric(&self, id: &str) -> Result<bool> {
        let existing = self.get_metric(id).await?;
        let Some(mut m) = existing else {
            return Ok(false);
        };
        m.status = "archived".to_string();
        self.insert_metric(m).await?;
        Ok(true)
    }

    // ── Events ────────────────────────────────────────────────────────────────

    pub async fn ingest_event(&self, event: Event) -> Result<()> {
        let mut builders = <Event as BuildRows>::new_builders(1);
        builders.append_rows(vec![event]);
        let batch = builders.finish().into_record_batch();
        self.events.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest event: {e}"))
    }

    pub async fn ingest_events(&self, events: Vec<Event>) -> Result<()> {
        if events.is_empty() { return Ok(()); }
        let n = events.len();
        let mut builders = <Event as BuildRows>::new_builders(n);
        builders.append_rows(events);
        let batch = builders.finish().into_record_batch();
        self.events.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest events: {e}"))
    }

    // ── Subscriptions ─────────────────────────────────────────────────────────

    pub async fn insert_subscription(&self, sub: Subscription) -> Result<()> {
        let mut builders = <Subscription as BuildRows>::new_builders(1);
        builders.append_rows(vec![sub]);
        let batch = builders.finish().into_record_batch();
        self.subscriptions.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest subscription: {e}"))
    }

    pub async fn subscription_exists(&self, id: &str) -> Result<bool> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.subscriptions.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan subscription {id}: {e}"))?;
        for batch in &batches {
            if batch.num_rows() > 0 { return Ok(true); }
        }
        Ok(false)
    }

    pub async fn get_subscription(&self, id: &str) -> Result<Option<Subscription>> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.subscriptions.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan subscription {id}: {e}"))?;
        for batch in &batches {
            let views = batch.iter_views::<Subscription>()
                .map_err(|e| anyhow::anyhow!("subscription schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("subscription view access: {e}"))?;
            for view in views {
                return Ok(Some(Subscription {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    status: view.status.to_string(),
                    current_period_start: view.current_period_start,
                    current_period_end: view.current_period_end,
                    stripe_subscription_id: view.stripe_subscription_id.to_string(),
                    created_at: view.created_at,
                    cancelled_at: view.cancelled_at,
                }));
            }
        }
        Ok(None)
    }

    pub async fn list_subscriptions_for_customer(&self, customer_id: &str) -> Result<Vec<Subscription>> {
        let filter = Expr::eq("customer_id", ScalarValue::Utf8(Some(customer_id.to_string())));
        let batches = self.subscriptions.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan subscriptions for customer {customer_id}: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Subscription>()
                .map_err(|e| anyhow::anyhow!("subscription schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("subscription view access: {e}"))?;
            for view in views {
                result.push(Subscription {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    status: view.status.to_string(),
                    current_period_start: view.current_period_start,
                    current_period_end: view.current_period_end,
                    stripe_subscription_id: view.stripe_subscription_id.to_string(),
                    created_at: view.created_at,
                    cancelled_at: view.cancelled_at,
                });
            }
        }
        Ok(result)
    }

    // ── Customers ─────────────────────────────────────────────────────────────

    pub async fn insert_customer(&self, customer: Customer) -> Result<()> {
        let mut builders = <Customer as BuildRows>::new_builders(1);
        builders.append_rows(vec![customer]);
        let batch = builders.finish().into_record_batch();
        self.customers.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest customer: {e}"))
    }

    pub async fn customer_exists(&self, id: &str) -> Result<bool> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.customers.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan customer {id}: {e}"))?;
        for batch in &batches {
            if batch.num_rows() > 0 { return Ok(true); }
        }
        Ok(false)
    }

    pub async fn get_customer(&self, id: &str) -> Result<Option<Customer>> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.customers.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan customer {id}: {e}"))?;
        for batch in &batches {
            let views = batch.iter_views::<Customer>()
                .map_err(|e| anyhow::anyhow!("customer schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("customer view access: {e}"))?;
            for view in views {
                return Ok(Some(Customer {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    email: view.email.to_string(),
                    stripe_customer_id: view.stripe_customer_id.to_string(),
                    metadata: view.metadata.to_string(),
                    created_at: view.created_at,
                }));
            }
        }
        Ok(None)
    }

    /// Returns `(page, has_more)`. Sorts by id, applies cursor after + limit.
    pub async fn list_customers(&self, limit: usize, after: Option<&str>) -> Result<(Vec<Customer>, bool)> {
        let batches = self.customers.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan customers: {e}"))?;
        let mut all: Vec<Customer> = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Customer>()
                .map_err(|e| anyhow::anyhow!("customer schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("customer view access: {e}"))?;
            for view in views {
                all.push(Customer {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    email: view.email.to_string(),
                    stripe_customer_id: view.stripe_customer_id.to_string(),
                    metadata: view.metadata.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        all.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(cursor) = after {
            all.retain(|c| c.id.as_str() > cursor);
        }
        let has_more = all.len() > limit;
        all.truncate(limit);
        Ok((all, has_more))
    }

    // ── Plans ─────────────────────────────────────────────────────────────────

    pub async fn insert_plan(&self, plan: Plan) -> Result<()> {
        let mut builders = <Plan as BuildRows>::new_builders(1);
        builders.append_rows(vec![plan]);
        let batch = builders.finish().into_record_batch();
        self.plans.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest plan: {e}"))
    }

    pub async fn plan_exists(&self, id: &str) -> Result<bool> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.plans.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan plan {id}: {e}"))?;
        for batch in &batches {
            if batch.num_rows() > 0 { return Ok(true); }
        }
        Ok(false)
    }

    pub async fn get_plan(&self, id: &str) -> Result<Option<Plan>> {
        let filter = Expr::eq("id", ScalarValue::Utf8(Some(id.to_string())));
        let batches = self.plans.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan plan {id}: {e}"))?;
        for batch in &batches {
            let views = batch.iter_views::<Plan>()
                .map_err(|e| anyhow::anyhow!("plan schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("plan view access: {e}"))?;
            for view in views {
                return Ok(Some(Plan {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    status: view.status.to_string(),
                    currency: view.currency.to_string(),
                    interval: view.interval.to_string(),
                    created_at: view.created_at,
                }));
            }
        }
        Ok(None)
    }

    pub async fn list_plans(&self, status: &str) -> Result<Vec<Plan>> {
        let filter = Expr::eq("status", ScalarValue::Utf8(Some(status.to_string())));
        let batches = self.plans.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan plans: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Plan>()
                .map_err(|e| anyhow::anyhow!("plan schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("plan view access: {e}"))?;
            for view in views {
                result.push(Plan {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    status: view.status.to_string(),
                    currency: view.currency.to_string(),
                    interval: view.interval.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    pub async fn archive_plan(&self, id: &str) -> Result<bool> {
        let Some(mut plan) = self.get_plan(id).await? else { return Ok(false); };
        if plan.status == "archived" { return Ok(false); }
        plan.status = "archived".to_string();
        self.insert_plan(plan).await?;
        Ok(true)
    }

    // ── Charges ───────────────────────────────────────────────────────────────

    pub async fn insert_charge(&self, charge: Charge) -> Result<()> {
        let mut builders = <Charge as BuildRows>::new_builders(1);
        builders.append_rows(vec![charge]);
        let batch = builders.finish().into_record_batch();
        self.charges.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest charge: {e}"))
    }

    pub async fn get_charges_for_plan(&self, plan_id: &str) -> Result<Vec<Charge>> {
        let filter = Expr::eq("plan_id", ScalarValue::Utf8(Some(plan_id.to_string())));
        let batches = self.charges.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan charges for plan {plan_id}: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Charge>()
                .map_err(|e| anyhow::anyhow!("charge schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("charge view access: {e}"))?;
            for view in views {
                result.push(Charge {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    metric: view.metric.to_string(),
                    model: view.model.to_string(),
                    unit_price: view.unit_price,
                    flat_amount: view.flat_amount,
                    package_size: view.package_size,
                    display: view.display.to_string(),
                });
            }
        }
        Ok(result)
    }

    // ── Tiers ─────────────────────────────────────────────────────────────────

    pub async fn insert_tier(&self, tier: Tier) -> Result<()> {
        let mut builders = <Tier as BuildRows>::new_builders(1);
        builders.append_rows(vec![tier]);
        let batch = builders.finish().into_record_batch();
        self.tiers.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest tier: {e}"))
    }

    pub async fn get_tiers_for_charge(&self, charge_id: &str) -> Result<Vec<Tier>> {
        let filter = Expr::eq("charge_id", ScalarValue::Utf8(Some(charge_id.to_string())));
        let batches = self.tiers.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan tiers for charge {charge_id}: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Tier>()
                .map_err(|e| anyhow::anyhow!("tier schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("tier view access: {e}"))?;
            for view in views {
                result.push(Tier {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    charge_id: view.charge_id.to_string(),
                    up_to: view.up_to,
                    unit_price: view.unit_price,
                    flat_fee: view.flat_fee,
                    position: view.position,
                });
            }
        }
        result.sort_by_key(|t| t.position);
        Ok(result)
    }

    // ── SQL Query via DataFusion ───────────────────────────────────────────────

    pub async fn run_query(&self, sql: &str) -> Result<QueryResult> {
        use datafusion::prelude::*;
        use datafusion::arrow::array::{
            Float64Array, Int32Array, Int64Array, StringArray,
        };
        use datafusion::arrow::datatypes::{DataType, Field, Schema};
        use datafusion::arrow::record_batch::RecordBatch as DfRecordBatch;
        use std::sync::Arc as StdArc;

        let ctx = SessionContext::new_with_config(
            datafusion::prelude::SessionConfig::new().with_information_schema(true),
        );

        // ── events ────────────────────────────────────────────────────────────
        {
            let rows = self.scan_events_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("customer_id", DataType::Utf8, false),
                Field::new("metric", DataType::Utf8, false),
                Field::new("quantity", DataType::Float64, false),
                Field::new("timestamp", DataType::Int64, false),
                Field::new("properties", DataType::Utf8, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.customer_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.metric.as_str()).collect::<Vec<_>>())),
                StdArc::new(Float64Array::from(rows.iter().map(|r| r.quantity).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.timestamp).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.properties.as_str()).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build events batch: {e}"))?;
            ctx.register_batch("events", batch)
                .map_err(|e| anyhow::anyhow!("register events: {e}"))?;
        }

        // ── customers ─────────────────────────────────────────────────────────
        {
            let rows = self.scan_customers_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("name", DataType::Utf8, false),
                Field::new("email", DataType::Utf8, false),
                Field::new("stripe_customer_id", DataType::Utf8, false),
                Field::new("metadata", DataType::Utf8, false),
                Field::new("created_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.email.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.stripe_customer_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.metadata.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build customers batch: {e}"))?;
            ctx.register_batch("customers", batch)
                .map_err(|e| anyhow::anyhow!("register customers: {e}"))?;
        }

        // ── metrics ───────────────────────────────────────────────────────────
        {
            let rows = self.scan_metrics_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("display", DataType::Utf8, false),
                Field::new("aggregation", DataType::Utf8, false),
                Field::new("status", DataType::Utf8, false),
                Field::new("created_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.display.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.aggregation.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build metrics batch: {e}"))?;
            ctx.register_batch("metrics", batch)
                .map_err(|e| anyhow::anyhow!("register metrics: {e}"))?;
        }

        // ── subscriptions ─────────────────────────────────────────────────────
        {
            let rows = self.scan_subscriptions_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("customer_id", DataType::Utf8, false),
                Field::new("plan_id", DataType::Utf8, false),
                Field::new("status", DataType::Utf8, false),
                Field::new("current_period_start", DataType::Int64, false),
                Field::new("current_period_end", DataType::Int64, false),
                Field::new("stripe_subscription_id", DataType::Utf8, false),
                Field::new("created_at", DataType::Int64, false),
                Field::new("cancelled_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.customer_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.plan_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.current_period_start).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.current_period_end).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.stripe_subscription_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.cancelled_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build subscriptions batch: {e}"))?;
            ctx.register_batch("subscriptions", batch)
                .map_err(|e| anyhow::anyhow!("register subscriptions: {e}"))?;
        }

        // ── plans ─────────────────────────────────────────────────────────────
        {
            let rows = self.scan_plans_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("name", DataType::Utf8, false),
                Field::new("status", DataType::Utf8, false),
                Field::new("currency", DataType::Utf8, false),
                Field::new("interval", DataType::Utf8, false),
                Field::new("created_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.currency.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.interval.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build plans batch: {e}"))?;
            ctx.register_batch("plans", batch)
                .map_err(|e| anyhow::anyhow!("register plans: {e}"))?;
        }

        // ── charges ───────────────────────────────────────────────────────────
        {
            let rows = self.scan_charges_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("plan_id", DataType::Utf8, false),
                Field::new("metric", DataType::Utf8, false),
                Field::new("model", DataType::Utf8, false),
                Field::new("unit_price", DataType::Int64, false),
                Field::new("flat_amount", DataType::Int64, false),
                Field::new("package_size", DataType::Int64, false),
                Field::new("display", DataType::Utf8, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.plan_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.metric.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.model.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.unit_price).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.flat_amount).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.package_size).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.display.as_str()).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build charges batch: {e}"))?;
            ctx.register_batch("charges", batch)
                .map_err(|e| anyhow::anyhow!("register charges: {e}"))?;
        }

        // ── tiers ─────────────────────────────────────────────────────────────
        {
            let rows = self.scan_tiers_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("charge_id", DataType::Utf8, false),
                Field::new("up_to", DataType::Int64, false),
                Field::new("unit_price", DataType::Int64, false),
                Field::new("flat_fee", DataType::Int64, false),
                Field::new("position", DataType::Int32, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.charge_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.up_to).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.unit_price).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.flat_fee).collect::<Vec<_>>())),
                StdArc::new(Int32Array::from(rows.iter().map(|r| r.position).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build tiers batch: {e}"))?;
            ctx.register_batch("tiers", batch)
                .map_err(|e| anyhow::anyhow!("register tiers: {e}"))?;
        }

        // ── billing_runs ──────────────────────────────────────────────────────
        {
            let rows = self.scan_billing_runs_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("customer_id", DataType::Utf8, false),
                Field::new("subscription_id", DataType::Utf8, false),
                Field::new("plan_id", DataType::Utf8, false),
                Field::new("period_start", DataType::Int64, false),
                Field::new("period_end", DataType::Int64, false),
                Field::new("status", DataType::Utf8, false),
                Field::new("invoice_id", DataType::Utf8, false),
                Field::new("created_at", DataType::Int64, false),
                Field::new("completed_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.customer_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.subscription_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.plan_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.period_start).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.period_end).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.invoice_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.completed_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build billing_runs batch: {e}"))?;
            ctx.register_batch("billing_runs", batch)
                .map_err(|e| anyhow::anyhow!("register billing_runs: {e}"))?;
        }

        // ── invoices ──────────────────────────────────────────────────────────
        {
            let rows = self.scan_invoices_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("customer_id", DataType::Utf8, false),
                Field::new("subscription_id", DataType::Utf8, false),
                Field::new("billing_run_id", DataType::Utf8, false),
                Field::new("stripe_invoice_id", DataType::Utf8, false),
                Field::new("status", DataType::Utf8, false),
                Field::new("currency", DataType::Utf8, false),
                Field::new("amount_cents", DataType::Int64, false),
                Field::new("period_start", DataType::Int64, false),
                Field::new("period_end", DataType::Int64, false),
                Field::new("created_at", DataType::Int64, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.customer_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.subscription_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.billing_run_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.stripe_invoice_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.currency.as_str()).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.amount_cents).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.period_start).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.period_end).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build invoices batch: {e}"))?;
            ctx.register_batch("invoices", batch)
                .map_err(|e| anyhow::anyhow!("register invoices: {e}"))?;
        }

        // ── invoice_line_items ────────────────────────────────────────────────
        {
            let rows = self.scan_invoice_line_items_raw().await?;
            let schema = StdArc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new("tenant_id", DataType::Utf8, false),
                Field::new("invoice_id", DataType::Utf8, false),
                Field::new("metric", DataType::Utf8, false),
                Field::new("description", DataType::Utf8, false),
                Field::new("quantity", DataType::Float64, false),
                Field::new("amount_cents", DataType::Int64, false),
                Field::new("model", DataType::Utf8, false),
            ]));
            let batch = DfRecordBatch::try_new(schema.clone(), vec![
                StdArc::new(StringArray::from(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.tenant_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.invoice_id.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.metric.as_str()).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.description.as_str()).collect::<Vec<_>>())),
                StdArc::new(Float64Array::from(rows.iter().map(|r| r.quantity).collect::<Vec<_>>())),
                StdArc::new(Int64Array::from(rows.iter().map(|r| r.amount_cents).collect::<Vec<_>>())),
                StdArc::new(StringArray::from(rows.iter().map(|r| r.model.as_str()).collect::<Vec<_>>())),
            ]).map_err(|e| anyhow::anyhow!("build invoice_line_items batch: {e}"))?;
            ctx.register_batch("invoice_line_items", batch)
                .map_err(|e| anyhow::anyhow!("register invoice_line_items: {e}"))?;
        }

        // ── Execute SQL ───────────────────────────────────────────────────────

        let df = ctx.sql(sql).await.map_err(|e| anyhow::anyhow!("sql parse/plan: {e}"))?;
        let batches = df.collect().await.map_err(|e| anyhow::anyhow!("sql execute: {e}"))?;

        // ── Convert results ───────────────────────────────────────────────────

        use datafusion::arrow::array::Array;
        use datafusion::arrow::datatypes::DataType as DT;

        if batches.is_empty() {
            return Ok(QueryResult { columns: vec![], rows: vec![], row_count: 0 });
        }

        let schema = batches[0].schema();
        let columns: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
        let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();

        for batch in &batches {
            let n = batch.num_rows();
            for row_i in 0..n {
                let mut row = Vec::with_capacity(columns.len());
                for col_i in 0..batch.num_columns() {
                    let col = batch.column(col_i);
                    let val = if col.is_null(row_i) {
                        serde_json::Value::Null
                    } else {
                        match col.data_type() {
                            DT::Utf8 | DT::LargeUtf8 => {
                                let arr = col.as_any().downcast_ref::<datafusion::arrow::array::StringArray>();
                                match arr {
                                    Some(a) => serde_json::Value::String(a.value(row_i).to_string()),
                                    None => serde_json::Value::String(format!("{:?}", col)),
                                }
                            }
                            DT::Int32 => {
                                let arr = col.as_any().downcast_ref::<Int32Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::Int64 => {
                                let arr = col.as_any().downcast_ref::<Int64Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::Float32 => {
                                let arr = col.as_any().downcast_ref::<datafusion::arrow::array::Float32Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::Float64 => {
                                let arr = col.as_any().downcast_ref::<Float64Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::Boolean => {
                                let arr = col.as_any().downcast_ref::<datafusion::arrow::array::BooleanArray>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::UInt64 => {
                                let arr = col.as_any().downcast_ref::<datafusion::arrow::array::UInt64Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            DT::UInt32 => {
                                let arr = col.as_any().downcast_ref::<datafusion::arrow::array::UInt32Array>().unwrap();
                                serde_json::json!(arr.value(row_i))
                            }
                            _ => {
                                // Fallback: use array display
                                use datafusion::arrow::util::display::array_value_to_string;
                                match array_value_to_string(col.as_ref(), row_i) {
                                    Ok(s) => serde_json::Value::String(s),
                                    Err(_) => serde_json::Value::String(format!("{:?}", col.data_type())),
                                }
                            }
                        }
                    };
                    row.push(val);
                }
                rows.push(row);
            }
        }

        let row_count = rows.len();
        Ok(QueryResult { columns, rows, row_count })
    }

    // ── Raw scan helpers for query ────────────────────────────────────────────

    async fn scan_events_raw(&self) -> Result<Vec<Event>> {
        let batches = self.events.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan events: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Event>()
                .map_err(|e| anyhow::anyhow!("event schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("event view access: {e}"))?;
            for view in views {
                result.push(Event {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    metric: view.metric.to_string(),
                    quantity: view.quantity,
                    timestamp: view.timestamp,
                    properties: view.properties.to_string(),
                });
            }
        }
        Ok(result)
    }

    async fn scan_customers_raw(&self) -> Result<Vec<Customer>> {
        let batches = self.customers.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan customers: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Customer>()
                .map_err(|e| anyhow::anyhow!("customer schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("customer view access: {e}"))?;
            for view in views {
                result.push(Customer {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    email: view.email.to_string(),
                    stripe_customer_id: view.stripe_customer_id.to_string(),
                    metadata: view.metadata.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    async fn scan_metrics_raw(&self) -> Result<Vec<Metric>> {
        let batches = self.metrics.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan metrics: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Metric>()
                .map_err(|e| anyhow::anyhow!("metric schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("metric view access: {e}"))?;
            for view in views {
                result.push(Metric {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    display: view.display.to_string(),
                    aggregation: view.aggregation.to_string(),
                    status: view.status.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    async fn scan_subscriptions_raw(&self) -> Result<Vec<Subscription>> {
        let batches = self.subscriptions.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan subscriptions: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Subscription>()
                .map_err(|e| anyhow::anyhow!("subscription schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("subscription view access: {e}"))?;
            for view in views {
                result.push(Subscription {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    status: view.status.to_string(),
                    current_period_start: view.current_period_start,
                    current_period_end: view.current_period_end,
                    stripe_subscription_id: view.stripe_subscription_id.to_string(),
                    created_at: view.created_at,
                    cancelled_at: view.cancelled_at,
                });
            }
        }
        Ok(result)
    }

    async fn scan_plans_raw(&self) -> Result<Vec<Plan>> {
        let batches = self.plans.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan plans: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Plan>()
                .map_err(|e| anyhow::anyhow!("plan schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("plan view access: {e}"))?;
            for view in views {
                result.push(Plan {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    name: view.name.to_string(),
                    status: view.status.to_string(),
                    currency: view.currency.to_string(),
                    interval: view.interval.to_string(),
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    async fn scan_charges_raw(&self) -> Result<Vec<Charge>> {
        let batches = self.charges.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan charges: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Charge>()
                .map_err(|e| anyhow::anyhow!("charge schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("charge view access: {e}"))?;
            for view in views {
                result.push(Charge {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    metric: view.metric.to_string(),
                    model: view.model.to_string(),
                    unit_price: view.unit_price,
                    flat_amount: view.flat_amount,
                    package_size: view.package_size,
                    display: view.display.to_string(),
                });
            }
        }
        Ok(result)
    }

    async fn scan_tiers_raw(&self) -> Result<Vec<Tier>> {
        let batches = self.tiers.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan tiers: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Tier>()
                .map_err(|e| anyhow::anyhow!("tier schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("tier view access: {e}"))?;
            for view in views {
                result.push(Tier {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    charge_id: view.charge_id.to_string(),
                    up_to: view.up_to,
                    unit_price: view.unit_price,
                    flat_fee: view.flat_fee,
                    position: view.position,
                });
            }
        }
        Ok(result)
    }

    async fn scan_billing_runs_raw(&self) -> Result<Vec<BillingRun>> {
        let batches = self.billing_runs.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan billing_runs: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<BillingRun>()
                .map_err(|e| anyhow::anyhow!("billing_run schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("billing_run view access: {e}"))?;
            for view in views {
                result.push(BillingRun {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    subscription_id: view.subscription_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    period_start: view.period_start,
                    period_end: view.period_end,
                    status: view.status.to_string(),
                    invoice_id: view.invoice_id.to_string(),
                    created_at: view.created_at,
                    completed_at: view.completed_at,
                });
            }
        }
        Ok(result)
    }

    async fn scan_invoices_raw(&self) -> Result<Vec<Invoice>> {
        let batches = self.invoices.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan invoices: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Invoice>()
                .map_err(|e| anyhow::anyhow!("invoice schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("invoice view access: {e}"))?;
            for view in views {
                result.push(Invoice {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    subscription_id: view.subscription_id.to_string(),
                    billing_run_id: view.billing_run_id.to_string(),
                    stripe_invoice_id: view.stripe_invoice_id.to_string(),
                    status: view.status.to_string(),
                    currency: view.currency.to_string(),
                    amount_cents: view.amount_cents,
                    period_start: view.period_start,
                    period_end: view.period_end,
                    created_at: view.created_at,
                });
            }
        }
        Ok(result)
    }

    // ── Billing Runs ──────────────────────────────────────────────────────────

    pub async fn list_active_subscriptions(&self) -> Result<Vec<Subscription>> {
        let filter = Expr::eq("status", ScalarValue::Utf8(Some("active".to_string())));
        let batches = self.subscriptions.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan active subscriptions: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<Subscription>()
                .map_err(|e| anyhow::anyhow!("subscription schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("subscription view access: {e}"))?;
            for view in views {
                result.push(Subscription {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    customer_id: view.customer_id.to_string(),
                    plan_id: view.plan_id.to_string(),
                    status: view.status.to_string(),
                    current_period_start: view.current_period_start,
                    current_period_end: view.current_period_end,
                    stripe_subscription_id: view.stripe_subscription_id.to_string(),
                    created_at: view.created_at,
                    cancelled_at: view.cancelled_at,
                });
            }
        }
        Ok(result)
    }

    pub async fn billing_run_exists_for_period(&self, subscription_id: &str, period_start: i64) -> Result<bool> {
        let filter = Expr::and(vec![
            Expr::eq("subscription_id", ScalarValue::Utf8(Some(subscription_id.to_string()))),
            Expr::eq("period_start", ScalarValue::Int64(Some(period_start))),
        ]);
        let batches = self.billing_runs.scan().filter(filter).collect().await
            .map_err(|e| anyhow::anyhow!("scan billing_run for period: {e}"))?;
        for batch in &batches {
            if batch.num_rows() > 0 { return Ok(true); }
        }
        Ok(false)
    }

    pub async fn insert_billing_run(&self, run: BillingRun) -> Result<()> {
        let mut builders = <BillingRun as BuildRows>::new_builders(1);
        builders.append_rows(vec![run]);
        let batch = builders.finish().into_record_batch();
        self.billing_runs.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest billing_run: {e}"))
    }

    pub async fn insert_invoice(&self, invoice: Invoice) -> Result<()> {
        let mut builders = <Invoice as BuildRows>::new_builders(1);
        builders.append_rows(vec![invoice]);
        let batch = builders.finish().into_record_batch();
        self.invoices.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest invoice: {e}"))
    }

    pub async fn insert_invoice_line_item(&self, item: InvoiceLineItem) -> Result<()> {
        let mut builders = <InvoiceLineItem as BuildRows>::new_builders(1);
        builders.append_rows(vec![item]);
        let batch = builders.finish().into_record_batch();
        self.invoice_line_items.ingest(batch).await.map_err(|e| anyhow::anyhow!("ingest invoice_line_item: {e}"))
    }

    async fn scan_invoice_line_items_raw(&self) -> Result<Vec<InvoiceLineItem>> {
        let batches = self.invoice_line_items.scan().collect().await
            .map_err(|e| anyhow::anyhow!("scan invoice_line_items: {e}"))?;
        let mut result = Vec::new();
        for batch in &batches {
            let views = batch.iter_views::<InvoiceLineItem>()
                .map_err(|e| anyhow::anyhow!("invoice_line_item schema mismatch: {e}"))?
                .try_flatten()
                .map_err(|e| anyhow::anyhow!("invoice_line_item view access: {e}"))?;
            for view in views {
                result.push(InvoiceLineItem {
                    id: view.id.to_string(),
                    tenant_id: view.tenant_id.to_string(),
                    invoice_id: view.invoice_id.to_string(),
                    metric: view.metric.to_string(),
                    description: view.description.to_string(),
                    quantity: view.quantity,
                    amount_cents: view.amount_cents,
                    model: view.model.to_string(),
                });
            }
        }
        Ok(result)
    }
}

async fn open_table<T: SchemaMeta>(
    data_dir: &std::path::Path,
    table_name: &str,
) -> Result<TonboDB> {
    let table_path = data_dir.join(table_name);
    DbBuilder::from_schema(T::schema())
        .map_err(|e: DbBuildError| anyhow::anyhow!("invalid schema for {table_name}: {e}"))?
        .on_disk(&table_path)
        .map_err(|e: DbBuildError| anyhow::anyhow!("failed to configure disk store for {table_name}: {e}"))?
        // NeverSeal prevents memtable sealing → no minor compaction → no WAL segment pruning.
        // This eliminates the "wal writer dropped ack" failure under write load.
        // Data durability is provided by the WAL; on restart the WAL is replayed.
        .with_seal_policy(Arc::new(NeverSeal))
        .open()
        .await
        .map_err(|e: DbBuildError| anyhow::anyhow!("failed to open table {table_name}: {e}"))
}
