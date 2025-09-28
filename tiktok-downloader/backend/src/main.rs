use axum::{
    extract::ConnectInfo,
    http::StatusCode,
    middleware as axum_middleware,
    response::Json,
    routing::{get, post},
    Router,
};
use tower_http::services::ServeDir;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod handlers;
mod middleware;
mod models;
mod services;
mod utils;

use config::AppConfig;
use handlers::*; // This imports all handlers including the new profile download handlers
use crate::middleware::{logging_middleware, security_headers_middleware};

#[tokio::main]
async fn main() {
    // Load configuration
    let config = AppConfig::from_env();
    
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tiktok_downloader_backend=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Create downloads directory if it doesn't exist
    tokio::fs::create_dir_all(&config.temp_dir)
        .await
        .expect("Failed to create downloads directory");

    // Log reCAPTCHA configuration status
    if config.is_recaptcha_enabled() {
        tracing::info!("🔒 reCAPTCHA protection is ENABLED");
    } else {
        tracing::warn!("⚠️ reCAPTCHA protection is DISABLED - set RECAPTCHA_SECRET_KEY environment variable to enable");
    }

    // Build our application with routes
    let app = Router::new()
        .route("/", get(health_check))
        .route("/api/health", get(health_check))
        // Single video endpoints
        .route("/api/video/info", post(get_video_info))
        .route("/api/video/download", post(download_video)) // Legacy endpoint - now streams instead of saving files
        .route("/api/video/stream", get(stream_video_download)) // Primary streaming endpoint
        .route("/api/video/audio-stream", get(stream_audio_download)) // NEW: Audio-only streaming endpoint
        // Profile download endpoints - Phase 1 & 2
        .route("/api/profile/info", post(get_profile_info))
        .route("/api/profile/download", post(download_profile_zip)) // Phase 1: Download all videos
        .route("/api/profile/download-selected", post(download_selected_profile_videos)) // Phase 2: Download selected videos
        .route("/api/profile/stream", get(stream_profile_zip))
        // Serve downloaded files (for backward compatibility)
        .nest_service("/api/downloads", ServeDir::new(&config.temp_dir))
        // Add middleware layers
        .layer(axum_middleware::from_fn(security_headers_middleware))
        .layer(axum_middleware::from_fn(logging_middleware))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Create socket address from config
    let addr: SocketAddr = config.socket_addr().parse()
        .expect("Invalid socket address from configuration");
    
    tracing::info!("🚀 TikTok Downloader Backend listening on {}", addr);
    tracing::info!("📁 Downloads directory: {}", config.temp_dir);
    tracing::info!("⚙️  Configuration loaded: {:?}", config);
    
    // Create the server with ConnectInfo to enable IP extraction
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>()
    ).await.unwrap();
}

async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    let config = AppConfig::from_env();
    Ok(Json(serde_json::json!({
        "status": "healthy",
        "service": "tiktok-downloader-backend",
        "version": "0.1.0",
        "recaptcha_enabled": config.is_recaptcha_enabled()
    })))
}
