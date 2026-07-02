use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tracing::{error, info};

const DASHBOARD_HTML: &str = include_str!("../dashboard.html");

#[derive(Clone)]
struct ProxyState {
    http: reqwest::Client,
    upstream: reqwest::Url,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dashboard=info".into()),
        )
        .json()
        .init();

    let upstream = std::env::var("DATEM_API_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let upstream = reqwest::Url::parse(&upstream).expect("DATEM_API_URL must be a valid URL");

    let port: u16 = std::env::var("DASHBOARD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);

    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;

    let state = ProxyState { http, upstream };

    let app = Router::new()
        .route("/", get(serve_dashboard))
        .fallback(proxy)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(port, "datem-dashboard listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn serve_dashboard() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], DASHBOARD_HTML)
}

async fn proxy(State(state): State<ProxyState>, req: Request) -> Response {
    let method = req.method().clone();
    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    let target = match state.upstream.join(path_and_query) {
        Ok(url) => url,
        Err(e) => {
            error!(error = %e, "failed to build upstream url");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let headers = req.headers().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            error!(error = %e, "failed to read request body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);

    let mut builder = state.http.request(reqwest_method, target);
    for (name, value) in headers.iter() {
        // Skip hop-by-hop / framing headers; reqwest recomputes these itself.
        if name == header::HOST || name == header::CONTENT_LENGTH {
            continue;
        }
        builder = builder.header(name, value);
    }
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    let upstream_resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "upstream request failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let status = upstream_resp.status();
    let mut resp_headers = HeaderMap::new();
    for (name, value) in upstream_resp.headers().iter() {
        if name == header::TRANSFER_ENCODING || name == header::CONNECTION {
            continue;
        }
        resp_headers.insert(name.clone(), value.clone());
    }

    let bytes = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            error!(error = %e, "failed to read upstream body");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    *response.headers_mut() = resp_headers;
    response
}
