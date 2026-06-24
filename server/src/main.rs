mod api;

use anyhow::Result;
use datem_core::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "datem=info".into()),
        )
        .json()
        .init();

    let config = Config::from_env()?;
    api::router::run(config).await
}
