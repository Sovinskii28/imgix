mod errors;
mod handlers;
mod state;
mod utils;

use crate::handlers::compress::MAX_FILE_SIZE;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    utils::storage::ensure_upload_dirs().await?;

    let state = state::AppState::new();
    let app = Router::new()
        .route("/health", get(handlers::health::health))
        .route("/images/compress", post(handlers::compress::compress))
        .layer(DefaultBodyLimit::max(MAX_FILE_SIZE * 2))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:3000").await?;

    tracing::info!("Server started on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}
