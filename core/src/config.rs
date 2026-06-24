use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: String,
    pub s3_bucket: Option<String>,
    pub s3_region: String,
    pub s3_endpoint: Option<String>,
    pub s3_prefix: String,
    pub api_key: String,
    pub stripe_key: Option<String>,
    pub stripe_webhook_secret: Option<String>,
    pub port: u16,
    pub run_mode: RunMode,
    pub billing_cron: String,
}

#[derive(Clone, Debug)]
pub enum RunMode {
    Server,
    LambdaIngest,
    LambdaBilling,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let s3_endpoint = optional("DATEM_S3_ENDPOINT");

        Ok(Self {
            data_dir: optional("DATEM_DATA_DIR").unwrap_or_else(|| "./data".into()),
            s3_bucket: optional("DATEM_S3_BUCKET"),
            s3_region: optional("DATEM_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            s3_endpoint,
            s3_prefix: optional("DATEM_S3_PREFIX").unwrap_or_else(|| "datem".into()),
            api_key: required("DATEM_API_KEY")?,
            stripe_key: optional("DATEM_STRIPE_KEY"),
            stripe_webhook_secret: optional("DATEM_STRIPE_WEBHOOK_SECRET"),
            port: optional("DATEM_PORT")
                .map(|v| v.parse::<u16>().context("DATEM_PORT must be a valid port number"))
                .transpose()?
                .unwrap_or(3000),
            run_mode: match optional("DATEM_RUN_MODE").as_deref() {
                Some("lambda-ingest") => RunMode::LambdaIngest,
                Some("lambda-billing") => RunMode::LambdaBilling,
                _ => RunMode::Server,
            },
            billing_cron: optional("DATEM_BILLING_CRON")
                .unwrap_or_else(|| "0 0 1 * *".into()),
        })
    }
}

fn required(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing required env var: {key}"))
}

fn optional(key: &str) -> Option<String> {
    std::env::var(key).ok()
}
