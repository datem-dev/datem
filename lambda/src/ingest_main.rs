mod ingest;

use aws_lambda_events::event::sqs::SqsEvent;
use datem_core::config::Config;
use datem_core::db::DbHandle;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use once_cell::sync::OnceCell;

static DB: OnceCell<DbHandle> = OnceCell::new();

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
    let db = DbHandle::start(config).await.expect("failed to start db actor");
    DB.set(db).unwrap_or_else(|_| panic!("db already initialized"));

    lambda_runtime::run(service_fn(|event: LambdaEvent<SqsEvent>| async {
        let db = DB.get().expect("db not initialized");
        ingest::handler(event, db)
            .await
            .map_err(|e| -> Error { format!("{e:#}").into() })
    }))
    .await
}
