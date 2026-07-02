use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::json;

#[derive(Parser)]
#[command(about = "Black-box HTTP load generator for the datem API. \
Assumes ./scripts/seed.sh has already populated fixture data.")]
struct Args {
    /// Base URL of the datem API, e.g. http://localhost:3000
    #[arg(long, env = "DATEM_API_URL", default_value = "http://localhost:3000")]
    api_url: String,

    #[arg(long, env = "DATEM_API_KEY", default_value = "dev-api-key")]
    api_key: String,

    #[arg(long, default_value_t = 10)]
    concurrency: usize,

    #[arg(long, default_value_t = 30)]
    duration_secs: u64,

    #[arg(long, value_enum, default_value_t = Workload::Mixed)]
    workload: Workload,

    /// Number of events per POST /ingest/batch request.
    #[arg(long, default_value_t = 100)]
    batch_size: usize,

    /// Metric name to query/ingest against (must already exist, see scripts/seed.sh).
    #[arg(long, default_value = "api_calls")]
    metric: String,

    /// Customer id to ingest against (must already exist, see scripts/seed.sh).
    #[arg(long, default_value = "cust_acme")]
    customer_id: String,

    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    format: OutputFormat,
}

#[derive(Clone, Copy, ValueEnum)]
enum Workload {
    Health,
    Metrics,
    MetricsGetOne,
    IngestOne,
    IngestBatch,
    Query,
    Mixed,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Endpoint {
    Health,
    MetricsList,
    MetricsGetOne,
    IngestOne,
    IngestBatch,
    Query,
}

impl Endpoint {
    fn label(&self) -> &'static str {
        match self {
            Endpoint::Health => "GET /health",
            Endpoint::MetricsList => "GET /metrics",
            Endpoint::MetricsGetOne => "GET /metrics/{id}",
            Endpoint::IngestOne => "POST /ingest",
            Endpoint::IngestBatch => "POST /ingest/batch",
            Endpoint::Query => "POST /query",
        }
    }
}

fn endpoints_for(workload: Workload) -> Vec<Endpoint> {
    // Expanded-by-weight list; workers cycle through it for a deterministic,
    // dependency-free weighted distribution (no `rand` crate needed).
    match workload {
        Workload::Health => vec![Endpoint::Health],
        Workload::Metrics => vec![Endpoint::MetricsList],
        Workload::MetricsGetOne => vec![Endpoint::MetricsGetOne],
        Workload::IngestOne => vec![Endpoint::IngestOne],
        Workload::IngestBatch => vec![Endpoint::IngestBatch],
        Workload::Query => vec![Endpoint::Query],
        Workload::Mixed => vec![
            Endpoint::MetricsList,
            Endpoint::MetricsList,
            Endpoint::MetricsList,
            Endpoint::MetricsGetOne,
            Endpoint::MetricsGetOne,
            Endpoint::Query,
            Endpoint::Query,
            Endpoint::IngestOne,
            Endpoint::IngestOne,
            Endpoint::IngestBatch,
        ],
    }
}

struct Sample {
    endpoint: Endpoint,
    elapsed: Duration,
    status: u16,
}

#[derive(Serialize)]
struct EndpointReport {
    label: String,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    ok: u64,
    err: u64,
    req_per_sec: f64,
}

static EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    warmup_check(&http, &args).await;

    let run_id = format!("bench-{}", ulid::Ulid::new());
    let endpoints = endpoints_for(args.workload);
    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);

    let mut handles = Vec::with_capacity(args.concurrency);
    for worker_id in 0..args.concurrency {
        let http = http.clone();
        let api_url = args.api_url.clone();
        let api_key = args.api_key.clone();
        let metric = args.metric.clone();
        let customer_id = args.customer_id.clone();
        let run_id = run_id.clone();
        let endpoints = endpoints.clone();
        let batch_size = args.batch_size;

        handles.push(tokio::spawn(async move {
            let mut samples = Vec::new();
            let mut i = worker_id;
            while Instant::now() < deadline {
                let endpoint = endpoints[i % endpoints.len()];
                i += 1;
                let start = Instant::now();
                let status = execute(
                    &http, &api_url, &api_key, endpoint, &metric, &customer_id, &run_id,
                    worker_id, batch_size,
                )
                .await;
                samples.push(Sample { endpoint, elapsed: start.elapsed(), status });
            }
            samples
        }));
    }

    let mut all_samples = Vec::new();
    for h in handles {
        all_samples.extend(h.await?);
    }

    let mut reports = Vec::new();
    for endpoint in dedup(&endpoints) {
        let subset: Vec<&Sample> = all_samples.iter().filter(|s| s.endpoint == endpoint).collect();
        if !subset.is_empty() {
            reports.push(build_report(endpoint.label().to_string(), &subset, args.duration_secs));
        }
    }
    if endpoints.len() > 1 {
        let all: Vec<&Sample> = all_samples.iter().collect();
        reports.push(build_report("TOTAL".to_string(), &all, args.duration_secs));
    }

    match args.format {
        OutputFormat::Table => print_table(&reports),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&reports)?),
    }

    Ok(())
}

fn dedup(endpoints: &[Endpoint]) -> Vec<Endpoint> {
    let mut seen = Vec::new();
    for e in endpoints {
        if !seen.contains(e) {
            seen.push(*e);
        }
    }
    seen
}

async fn warmup_check(http: &reqwest::Client, args: &Args) {
    let url = format!("{}/metrics/{}", args.api_url, args.metric);
    let resp = http.get(&url).bearer_auth(&args.api_key).send().await;
    let ok = matches!(&resp, Ok(r) if r.status().is_success());
    if !ok {
        eprintln!(
            "warning: metric '{}' not found at {} — run ./scripts/seed.sh {} first to load test data",
            args.metric, url, args.api_url
        );
    }
}

fn now_micros() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_micros() as i64
}

async fn execute(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    endpoint: Endpoint,
    metric: &str,
    customer_id: &str,
    run_id: &str,
    worker_id: usize,
    batch_size: usize,
) -> u16 {
    let result = match endpoint {
        Endpoint::Health => http.get(format!("{api_url}/health")).send().await,
        Endpoint::MetricsList => http
            .get(format!("{api_url}/metrics"))
            .bearer_auth(api_key)
            .send()
            .await,
        Endpoint::MetricsGetOne => http
            .get(format!("{api_url}/metrics/{metric}"))
            .bearer_auth(api_key)
            .send()
            .await,
        Endpoint::IngestOne => {
            let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
            let body = json!({
                "event_id": format!("{run_id}-w{worker_id}-{seq}"),
                "customer_id": customer_id,
                "metric": metric,
                "quantity": 1,
                "timestamp": now_micros(),
            });
            http.post(format!("{api_url}/ingest"))
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
        }
        Endpoint::IngestBatch => {
            let events: Vec<_> = (0..batch_size)
                .map(|_| {
                    let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
                    json!({
                        "event_id": format!("{run_id}-w{worker_id}-{seq}"),
                        "customer_id": customer_id,
                        "metric": metric,
                        "quantity": 1,
                        "timestamp": now_micros(),
                    })
                })
                .collect();
            http.post(format!("{api_url}/ingest/batch"))
                .bearer_auth(api_key)
                .json(&json!({ "events": events }))
                .send()
                .await
        }
        Endpoint::Query => http
            .post(format!("{api_url}/query"))
            .bearer_auth(api_key)
            .json(&json!({ "sql": "SELECT COUNT(*) FROM events" }))
            .send()
            .await,
    };

    match result {
        Ok(resp) => resp.status().as_u16(),
        Err(_) => 0,
    }
}

fn build_report(label: String, samples: &[&Sample], duration_secs: u64) -> EndpointReport {
    let mut durations: Vec<f64> = samples.iter().map(|s| s.elapsed.as_secs_f64() * 1000.0).collect();
    durations.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let percentile = |p: f64| -> f64 {
        if durations.is_empty() {
            return 0.0;
        }
        let idx = ((durations.len() as f64 * p).ceil() as usize).saturating_sub(1);
        durations[idx.min(durations.len() - 1)]
    };

    let ok = samples.iter().filter(|s| (200..300).contains(&s.status)).count() as u64;
    let err = samples.len() as u64 - ok;

    EndpointReport {
        label,
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        ok,
        err,
        req_per_sec: samples.len() as f64 / duration_secs as f64,
    }
}

fn print_table(reports: &[EndpointReport]) {
    println!(
        "{:<20}  {:>8}  {:>8}  {:>8}  {:>10}  {:>6}  {:>6}",
        "Endpoint", "P50 (ms)", "P95 (ms)", "P99 (ms)", "Req/s", "OK", "Errors"
    );
    println!("{}", "-".repeat(76));
    for r in reports {
        println!(
            "{:<20}  {:>8.1}  {:>8.1}  {:>8.1}  {:>10.1}  {:>6}  {:>6}",
            r.label, r.p50_ms, r.p95_ms, r.p99_ms, r.req_per_sec, r.ok, r.err
        );
    }
}
