//! Minimal app: one API route plus the SPA bundle.
//!
//! Run with `cargo run --example serve`, then open http://127.0.0.1:3000.
//!
//! Debug builds proxy the SPA to `WEB_DEV_URL` (default
//! `http://127.0.0.1:5173`); start a Vite dev server there first. Release
//! builds serve the `examples/vite-app/dist/` folder (built by `bun run build`) embedded in the binary.

use axum::{Json, Router, routing::get};
use axum_vite_spa_bundle::{SpaConfig, spa_router};

#[derive(Clone, Copy, rust_embed::Embed)]
#[folder = "examples/vite-app/dist/"]
struct Web;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = Router::new()
        .route("/api/hello", get(|| async { Json("hello") }))
        // Merged last so the API route above keeps priority.
        .merge(spa_router(SpaConfig::new(
            Web,
            "WEB_DEV_URL",
            "http://127.0.0.1:5173",
            ["/api/"],
        )));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await
}
