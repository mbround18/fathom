//! `depths`: the Depths server binary (spec 050). A thin layer that stitches the shared crates
//! together; the logic lives in `crates/depths-*`.
//!
//! Placeholder until T042: serves `/healthz` and the web assets from `STATIC_DIR` so the image
//! (T076) can be built and health-checked ahead of Phase 3. No sign-in, no Docker access.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{Router, routing::get};
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listen: SocketAddr = std::env::var("DEPTHS_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .map_err(|e| std::io::Error::other(format!("DEPTHS_LISTEN: {e}")))?;
    let static_dir = PathBuf::from(
        std::env::var("STATIC_DIR").unwrap_or_else(|_| "/usr/share/depths/web".into()),
    );

    let spa = ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .fallback_service(spa);

    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!(
        "depths (placeholder) listening on {listen}, static {}",
        static_dir.display()
    );
    axum::serve(listener, app).await
}
