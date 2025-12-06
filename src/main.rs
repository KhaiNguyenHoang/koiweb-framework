//! Main application entry point for the Koiweb framework example.
//! 
//! This executable demonstrates how to use the `koiweb-framework`
//! to create a simple web server with routes, middleware, and file upload
//! functionality.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use futures::StreamExt;
use hyper::{Method, StatusCode};
use koiweb_framework::{App, AppError, AppResult, Middleware, Request, Response};
use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

// -----------------------------
// Middleware
// -----------------------------

/// A rate limiting middleware that limits requests based on the client's IP address.
///
/// It uses a token bucket algorithm to allow a maximum number of requests (`max_tokens`)
/// within a `refill_secs` period.
fn rate_limiter_middleware(
    state: Arc<DashMap<String, (usize, Instant)>>,
    max_tokens: usize,
    refill_secs: u64,
) -> Box<Middleware> {
    Box::new(move |req, next| {
        let state = state.clone();
        Box::pin(async move {
            let key = req
                .remote_addr
                .map(|a| a.ip().to_string())
                .unwrap_or_else(|| "anon".into());

            let mut entry = state.entry(key).or_insert((max_tokens, Instant::now()));
            let (tokens, last) = entry.value_mut();

            if last.elapsed().as_secs() >= refill_secs {
                *tokens = max_tokens;
                *last = Instant::now();
            }

            if *tokens == 0 {
                return Err(AppError::TooManyRequests);
            }
            *tokens -= 1;
            drop(entry);

            (next)(req).await
        })
    })
}

// -----------------------------
// File Upload
// -----------------------------

/// Handles file uploads.
///
/// Saves the uploaded file to the `uploads/` directory with a unique filename
/// based on the current timestamp. Limits file size to 10MB.
async fn handle_file_upload(req: Request) -> AppResult {
    let filename = format!(
        "uploads/upload-{}.bin",
        chrono::Utc::now().timestamp_millis()
    );
    let mut file = File::create(&filename).await.map_err(AppError::Io)?;

    let mut body = req.body;
    let mut total_bytes = 0;

    while let Some(chunk_result) = body.next().await {
        let chunk = chunk_result.map_err(|e| AppError::Internal(e.to_string()))?;
        file.write_all(&chunk).await.map_err(AppError::Io)?;
        total_bytes += chunk.len();

        if total_bytes > 10 * 1024 * 1024 {
            return Err(AppError::BadRequest("File too large".into()));
        }
    }

    Response::ok_json(&json!({
        "status": "uploaded",
        "path": filename,
        "size": total_bytes
    }))
}

/// The main entry point of the web server application.
///
/// Initializes the application, sets up routes, middleware, and starts the server.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    tokio::fs::create_dir_all("public").await?;
    tokio::fs::create_dir_all("uploads").await?;
    if !Path::new("public/index.html").exists() {
        tokio::fs::write("public/index.html", "<h1>Secure Server</h1>").await?;
    }

    let mut app = App::new();

    // Add rate limiting middleware
    let rl_state = Arc::new(DashMap::new());
    app = app.with_middleware(rate_limiter_middleware(rl_state, 50, 60));

    // Define routes
    app.route(Method::GET, "/", |_: Request| async move {
        Ok(Response::new(StatusCode::OK, "Home"))
    });

    app.route(Method::GET, "/users/:id", |req: Request| async move {
        let id = req.params.get("id").cloned().unwrap_or_default();
        Response::ok_json(&json!({"user_id": id}))
    });

    app.route(Method::POST, "/upload", handle_file_upload);

    // Grouped routes
    app.group("/api/v1")
        .route(Method::GET, "/test", |_: Request| async move {
            Response::ok_json(&json!({"status": "ok"}))
        });

    // Start the server
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("127.0.0.1:{}", port);
    app.run(&addr, None).await
}
