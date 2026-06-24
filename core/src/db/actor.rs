use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use crate::config::Config;
use super::{
    QueryResult,
    store::DatemStore,
    tables::{
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
    },
};

type Reply<T> = oneshot::Sender<Result<T>>;

/// Messages the actor loop can handle. Each variant carries a oneshot sender
/// so the caller can await the result without sharing ownership of the store.
pub(crate) enum DbMsg {
    // ── Metrics ──────────────────────────────────────────────────────────────
    InsertMetric(Metric, Reply<()>),
    MetricExists(String, Reply<bool>),
    GetMetric(String, Reply<Option<Metric>>),
    ListMetrics(Reply<Vec<Metric>>),
    ArchiveMetric(String, Reply<bool>),
    // ── Events ────────────────────────────────────────────────────────────────
    IngestEvent(Event, Reply<()>),
    IngestEvents(Vec<Event>, Reply<()>),
    // ── Subscriptions ─────────────────────────────────────────────────────────
    InsertSubscription(Subscription, Reply<()>),
    SubscriptionExists(String, Reply<bool>),
    GetSubscription(String, Reply<Option<Subscription>>),
    ListSubscriptionsForCustomer(String, Reply<Vec<Subscription>>),
    // ── Customers ─────────────────────────────────────────────────────────────
    InsertCustomer(Customer, Reply<()>),
    CustomerExists(String, Reply<bool>),
    GetCustomer(String, Reply<Option<Customer>>),
    ListCustomers(usize, Option<String>, Reply<(Vec<Customer>, bool)>),
    // ── Plans ─────────────────────────────────────────────────────────────────
    InsertPlan(Plan, Reply<()>),
    PlanExists(String, Reply<bool>),
    GetPlan(String, Reply<Option<Plan>>),
    ListPlans(String, Reply<Vec<Plan>>),
    ArchivePlan(String, Reply<bool>),
    // ── Charges ───────────────────────────────────────────────────────────────
    InsertCharge(Charge, Reply<()>),
    GetChargesForPlan(String, Reply<Vec<Charge>>),
    // ── Tiers ─────────────────────────────────────────────────────────────────
    InsertTier(Tier, Reply<()>),
    GetTiersForCharge(String, Reply<Vec<Tier>>),
    // ── Billing Runs ──────────────────────────────────────────────────────────
    ListActiveSubscriptions(Reply<Vec<Subscription>>),
    BillingRunExistsForPeriod(String, i64, Reply<bool>),
    InsertBillingRun(BillingRun, Reply<()>),
    InsertInvoice(Invoice, Reply<()>),
    InsertInvoiceLineItem(InvoiceLineItem, Reply<()>),
    // ── SQL Query ─────────────────────────────────────────────────────────────
    RunQuery(String, Reply<QueryResult>),
}

/// A cheap, `Clone + Send + Sync` handle to the DB actor.
///
/// All methods send a message to the actor and wait for the response via a
/// oneshot channel. The actor loop runs inside `rt.block_on()` on a dedicated
/// OS thread, so tonbo's `!Send` scan streams never cross thread boundaries or
/// appear in axum's `Send` handler futures. A multi-thread runtime is required
/// because `LocalFs` (TokioFs) calls `block_in_place` for disk I/O, which
/// panics on a current-thread runtime.
#[derive(Clone)]
pub struct DbHandle {
    tx: mpsc::Sender<DbMsg>,
}

impl DbHandle {
    /// Spawn the DB actor on a dedicated OS thread and return a handle.
    pub async fn start(config: Config) -> Result<Self> {
        let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();
        let (msg_tx, msg_rx) = mpsc::channel::<DbMsg>(256);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("db actor: failed to build runtime");

            rt.block_on(async {
                match DatemStore::new(&config).await {
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                    Ok(store) => {
                        let _ = ready_tx.send(Ok(()));
                        actor_loop(store, msg_rx).await;
                    }
                }
            });
        });

        ready_rx.await??;
        Ok(Self { tx: msg_tx })
    }

    // ── Metrics ──────────────────────────────────────────────────────────────

    pub async fn insert_metric(&self, metric: Metric) -> Result<()> {
        self.send(|reply| DbMsg::InsertMetric(metric, reply)).await
    }

    pub async fn metric_exists(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::MetricExists(id.to_string(), reply)).await
    }

    pub async fn get_metric(&self, id: &str) -> Result<Option<Metric>> {
        self.send(|reply| DbMsg::GetMetric(id.to_string(), reply)).await
    }

    pub async fn list_metrics(&self) -> Result<Vec<Metric>> {
        self.send(|reply| DbMsg::ListMetrics(reply)).await
    }

    pub async fn archive_metric(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::ArchiveMetric(id.to_string(), reply)).await
    }

    // ── Events ────────────────────────────────────────────────────────────────

    pub async fn ingest_event(&self, event: Event) -> Result<()> {
        self.send(|reply| DbMsg::IngestEvent(event, reply)).await
    }

    pub async fn ingest_events(&self, events: Vec<Event>) -> Result<()> {
        self.send(|reply| DbMsg::IngestEvents(events, reply)).await
    }

    // ── Subscriptions ─────────────────────────────────────────────────────────

    pub async fn insert_subscription(&self, sub: Subscription) -> Result<()> {
        self.send(|reply| DbMsg::InsertSubscription(sub, reply)).await
    }

    pub async fn subscription_exists(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::SubscriptionExists(id.to_string(), reply)).await
    }

    pub async fn get_subscription(&self, id: &str) -> Result<Option<Subscription>> {
        self.send(|reply| DbMsg::GetSubscription(id.to_string(), reply)).await
    }

    pub async fn list_subscriptions_for_customer(&self, customer_id: &str) -> Result<Vec<Subscription>> {
        self.send(|reply| DbMsg::ListSubscriptionsForCustomer(customer_id.to_string(), reply)).await
    }

    // ── Customers ─────────────────────────────────────────────────────────────

    pub async fn insert_customer(&self, customer: Customer) -> Result<()> {
        self.send(|reply| DbMsg::InsertCustomer(customer, reply)).await
    }

    pub async fn customer_exists(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::CustomerExists(id.to_string(), reply)).await
    }

    pub async fn get_customer(&self, id: &str) -> Result<Option<Customer>> {
        self.send(|reply| DbMsg::GetCustomer(id.to_string(), reply)).await
    }

    pub async fn list_customers(&self, limit: usize, after: Option<&str>) -> Result<(Vec<Customer>, bool)> {
        self.send(|reply| DbMsg::ListCustomers(limit, after.map(|s| s.to_string()), reply)).await
    }

    // ── Plans ─────────────────────────────────────────────────────────────────

    pub async fn insert_plan(&self, plan: Plan) -> Result<()> {
        self.send(|reply| DbMsg::InsertPlan(plan, reply)).await
    }

    pub async fn plan_exists(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::PlanExists(id.to_string(), reply)).await
    }

    pub async fn get_plan(&self, id: &str) -> Result<Option<Plan>> {
        self.send(|reply| DbMsg::GetPlan(id.to_string(), reply)).await
    }

    pub async fn list_plans(&self, status: &str) -> Result<Vec<Plan>> {
        self.send(|reply| DbMsg::ListPlans(status.to_string(), reply)).await
    }

    pub async fn archive_plan(&self, id: &str) -> Result<bool> {
        self.send(|reply| DbMsg::ArchivePlan(id.to_string(), reply)).await
    }

    // ── Charges ───────────────────────────────────────────────────────────────

    pub async fn insert_charge(&self, charge: Charge) -> Result<()> {
        self.send(|reply| DbMsg::InsertCharge(charge, reply)).await
    }

    pub async fn get_charges_for_plan(&self, plan_id: &str) -> Result<Vec<Charge>> {
        self.send(|reply| DbMsg::GetChargesForPlan(plan_id.to_string(), reply)).await
    }

    // ── Tiers ─────────────────────────────────────────────────────────────────

    pub async fn insert_tier(&self, tier: Tier) -> Result<()> {
        self.send(|reply| DbMsg::InsertTier(tier, reply)).await
    }

    pub async fn get_tiers_for_charge(&self, charge_id: &str) -> Result<Vec<Tier>> {
        self.send(|reply| DbMsg::GetTiersForCharge(charge_id.to_string(), reply)).await
    }

    // ── Billing Runs ──────────────────────────────────────────────────────────

    pub async fn list_active_subscriptions(&self) -> Result<Vec<Subscription>> {
        self.send(|reply| DbMsg::ListActiveSubscriptions(reply)).await
    }

    pub async fn billing_run_exists_for_period(&self, subscription_id: &str, period_start: i64) -> Result<bool> {
        self.send(|reply| DbMsg::BillingRunExistsForPeriod(subscription_id.to_string(), period_start, reply)).await
    }

    pub async fn insert_billing_run(&self, run: BillingRun) -> Result<()> {
        self.send(|reply| DbMsg::InsertBillingRun(run, reply)).await
    }

    pub async fn insert_invoice(&self, invoice: Invoice) -> Result<()> {
        self.send(|reply| DbMsg::InsertInvoice(invoice, reply)).await
    }

    pub async fn insert_invoice_line_item(&self, item: InvoiceLineItem) -> Result<()> {
        self.send(|reply| DbMsg::InsertInvoiceLineItem(item, reply)).await
    }

    // ── SQL Query ─────────────────────────────────────────────────────────────

    pub async fn run_query(&self, sql: String) -> Result<QueryResult> {
        self.send(|reply| DbMsg::RunQuery(sql, reply)).await
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    async fn send<T, F>(&self, build: F) -> Result<T>
    where
        F: FnOnce(Reply<T>) -> DbMsg,
        T: Send + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(build(reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("db actor disconnected"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("db actor reply dropped"))?
    }
}

async fn actor_loop(store: Arc<DatemStore>, mut rx: mpsc::Receiver<DbMsg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            DbMsg::InsertMetric(metric, reply) => {
                let _ = reply.send(store.insert_metric(metric).await);
            }
            DbMsg::MetricExists(id, reply) => {
                let _ = reply.send(store.metric_exists(&id).await);
            }
            DbMsg::GetMetric(id, reply) => {
                let _ = reply.send(store.get_metric(&id).await);
            }
            DbMsg::ListMetrics(reply) => {
                let _ = reply.send(store.list_metrics().await);
            }
            DbMsg::ArchiveMetric(id, reply) => {
                let _ = reply.send(store.archive_metric(&id).await);
            }
            DbMsg::IngestEvent(event, reply) => {
                let _ = reply.send(store.ingest_event(event).await);
            }
            DbMsg::IngestEvents(events, reply) => {
                let _ = reply.send(store.ingest_events(events).await);
            }
            DbMsg::InsertSubscription(sub, reply) => {
                let _ = reply.send(store.insert_subscription(sub).await);
            }
            DbMsg::SubscriptionExists(id, reply) => {
                let _ = reply.send(store.subscription_exists(&id).await);
            }
            DbMsg::GetSubscription(id, reply) => {
                let _ = reply.send(store.get_subscription(&id).await);
            }
            DbMsg::ListSubscriptionsForCustomer(customer_id, reply) => {
                let _ = reply.send(store.list_subscriptions_for_customer(&customer_id).await);
            }
            DbMsg::InsertCustomer(customer, reply) => {
                let _ = reply.send(store.insert_customer(customer).await);
            }
            DbMsg::CustomerExists(id, reply) => {
                let _ = reply.send(store.customer_exists(&id).await);
            }
            DbMsg::GetCustomer(id, reply) => {
                let _ = reply.send(store.get_customer(&id).await);
            }
            DbMsg::ListCustomers(limit, after, reply) => {
                let _ = reply.send(store.list_customers(limit, after.as_deref()).await);
            }
            DbMsg::InsertPlan(plan, reply) => {
                let _ = reply.send(store.insert_plan(plan).await);
            }
            DbMsg::PlanExists(id, reply) => {
                let _ = reply.send(store.plan_exists(&id).await);
            }
            DbMsg::GetPlan(id, reply) => {
                let _ = reply.send(store.get_plan(&id).await);
            }
            DbMsg::ListPlans(status, reply) => {
                let _ = reply.send(store.list_plans(&status).await);
            }
            DbMsg::ArchivePlan(id, reply) => {
                let _ = reply.send(store.archive_plan(&id).await);
            }
            DbMsg::InsertCharge(charge, reply) => {
                let _ = reply.send(store.insert_charge(charge).await);
            }
            DbMsg::GetChargesForPlan(plan_id, reply) => {
                let _ = reply.send(store.get_charges_for_plan(&plan_id).await);
            }
            DbMsg::InsertTier(tier, reply) => {
                let _ = reply.send(store.insert_tier(tier).await);
            }
            DbMsg::GetTiersForCharge(charge_id, reply) => {
                let _ = reply.send(store.get_tiers_for_charge(&charge_id).await);
            }
            DbMsg::ListActiveSubscriptions(reply) => {
                let _ = reply.send(store.list_active_subscriptions().await);
            }
            DbMsg::BillingRunExistsForPeriod(sub_id, period_start, reply) => {
                let _ = reply.send(store.billing_run_exists_for_period(&sub_id, period_start).await);
            }
            DbMsg::InsertBillingRun(run, reply) => {
                let _ = reply.send(store.insert_billing_run(run).await);
            }
            DbMsg::InsertInvoice(invoice, reply) => {
                let _ = reply.send(store.insert_invoice(invoice).await);
            }
            DbMsg::InsertInvoiceLineItem(item, reply) => {
                let _ = reply.send(store.insert_invoice_line_item(item).await);
            }
            DbMsg::RunQuery(sql, reply) => {
                let _ = reply.send(store.run_query(&sql).await);
            }
        }
    }
}
