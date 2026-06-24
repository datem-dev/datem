mod billing;

use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use datem_core::config::Config;
use datem_core::db::DbHandle;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use once_cell::sync::OnceCell;

static DB: OnceCell<DbHandle> = OnceCell::new();
static CONFIG: OnceCell<Config> = OnceCell::new();

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "datem_lambda=info".into()),
        )
        .json()
        .without_time()
        .init();

    let config = Config::from_env().expect("failed to load config");
    let db = DbHandle::start(config.clone()).await.expect("failed to start db actor");
    DB.set(db).unwrap_or_else(|_| panic!("db already initialized"));
    CONFIG.set(config).unwrap_or_else(|_| panic!("config already initialized"));

    lambda_runtime::run(service_fn(
        |event: LambdaEvent<EventBridgeEvent<serde_json::Value>>| async {
            let db = DB.get().expect("db not initialized");
            let config = CONFIG.get().expect("config not initialized");
            billing::handler(event, db, config)
                .await
                .map_err(|e| -> Error { format!("{e:#}").into() })
        },
    ))
    .await
}
