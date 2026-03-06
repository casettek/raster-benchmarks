use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use tower_http::services::ServeDir;

/// Serves the `web/` directory as static files.
///
/// Run from the repo root:
///
///   cargo run --manifest-path apps/web-server/Cargo.toml
///
/// Then open: http://localhost:3030
///
/// The `web/` directory is resolved relative to the current working directory,
/// so this must be run from the repo root.
#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8010);

    let web_dir = PathBuf::from("web");

    if !web_dir.exists() {
        eprintln!(
            "error: `web/` directory not found. Run this from the repo root:\n  \
             cargo run --manifest-path apps/web-server/Cargo.toml"
        );
        std::process::exit(1);
    }

    let app = Router::new().nest_service("/", ServeDir::new(web_dir));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("serving web/ at http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
