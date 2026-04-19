mod archives;
mod article_states;
mod categories;
mod clerk;
mod error;
mod etag;
mod jwks;
mod prefs;
mod profile;
mod rate_limit;
mod sources;
mod state;
mod types;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use axum::routing::get;
use axum::{middleware, Router};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::jwks::JwksCache;
use crate::rate_limit::RateLimiter;
use crate::state::AppState;

/// Hard cap on request bodies — archives are the largest legitimate payload
/// (~8 MiB per `archives.rs`). This sits in front of handler-level size
/// validation to keep garbage bodies from burning CPU in serde.
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load radar-server/.env regardless of CWD — CARGO_MANIFEST_DIR is the
    // crate root at compile time, so `cargo run -p radar-server` works from
    // anywhere in the workspace.
    let _ = dotenvy::from_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env"),
    );
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "radar_server=info,tower_http=info".into()),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set")?;
    // Port resolution: `PORT` is the de-facto PaaS convention (Render, Fly,
    // Heroku, Railway all set this automatically). `SERVER_PORT` is our
    // local-dev override; 8787 is the final fallback.
    let port: u16 = std::env::var("PORT")
        .or_else(|_| std::env::var("SERVER_PORT"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8787);
    let clerk_jwks_url =
        std::env::var("CLERK_JWKS_URL").map_err(|_| "CLERK_JWKS_URL not set")?;
    let clerk_issuer =
        std::env::var("CLERK_ISSUER").map_err(|_| "CLERK_ISSUER not set")?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;

    sqlx::migrate!().run(&db).await?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let state = AppState {
        db,
        jwks: JwksCache::new(http, clerk_jwks_url),
        clerk_issuer: Arc::new(clerk_issuer),
    };

    // Per-user token bucket. Capacity 40 covers bursty editing (typing +
    // multiple resource syncs); 10 rps sustained is more than any well-
    // behaved client will ever hit.
    let limiter = RateLimiter::new(40, 10);

    // Everything under /sync/* requires a valid Clerk session JWT.
    // Order: session_layer first (populates ClerkUser), then rate_limit reads it.
    let authed = Router::new()
        .route(
            "/sync/profile",
            get(profile::get_profile).put(profile::put_profile),
        )
        .route(
            "/sync/categories",
            get(categories::get_categories).put(categories::put_categories),
        )
        .route(
            "/sync/sources",
            get(sources::get_sources).put(sources::put_sources),
        )
        .route(
            "/sync/archives",
            get(archives::get_archives).put(archives::put_archives),
        )
        .route(
            "/sync/article-states",
            get(article_states::get_article_states)
                .patch(article_states::patch_article_state),
        )
        .route(
            "/sync/prefs",
            get(prefs::get_prefs).put(prefs::put_prefs),
        )
        .layer(middleware::from_fn_with_state(
            limiter.clone(),
            rate_limit::layer,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            clerk::session_layer,
        ));

    let cors = build_cors()?;

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(authed)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("radar-server listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Build the CORS layer from the `CORS_ORIGINS` env var — comma-separated
/// list. Defaults to the Tauri dev origins if unset so local development
/// just works. Exposes `ETag` so the browser can read it from fetch
/// responses (it's not in the CORS-safe-list by default).
fn build_cors() -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let raw = std::env::var("CORS_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:1420,tauri://localhost".into());
    let origins: Vec<HeaderValue> = raw
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<HeaderValue>()
                .map_err(|e| format!("invalid CORS_ORIGINS entry '{s}': {e}"))
        })
        .collect::<Result<_, _>>()?;

    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::PUT, Method::PATCH, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::IF_MATCH,
            header::IF_NONE_MATCH,
        ])
        .expose_headers([header::ETAG])
        .max_age(Duration::from_secs(600)))
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install ^C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
