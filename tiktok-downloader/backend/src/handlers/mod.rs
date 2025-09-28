use axum::{
    extract::{Json, Query, ConnectInfo},
    http::{StatusCode, header::{CONTENT_TYPE, CONTENT_DISPOSITION}},
    response::{IntoResponse, Response},
    body::Body,
};
use std::{path::PathBuf, net::SocketAddr};

// STREAMING REFACTOR COMPLETE:
// All download endpoints now use direct streaming (yt-dlp stdout -> browser)
// No server disk usage - zero file creation - instant downloads
use futures_util::TryStreamExt;
use serde_json::json;
use tokio_util::io::ReaderStream;
use tokio::fs::File;

use crate::models::*;
use crate::services::{TikTokService, RecaptchaService};
use crate::config::AppConfig;

// Helper function to create services with reCAPTCHA support
fn create_recaptcha_service() -> RecaptchaService {
    let config = AppConfig::from_env();
    RecaptchaService::new(config.recaptcha_secret_key)
}

// Helper function to verify reCAPTCHA token if provided and enabled
async fn verify_recaptcha_if_enabled(
    recaptcha_token: Option<&String>,
    client_ip: Option<String>,
) -> Result<(), AppError> {
    let recaptcha_service = create_recaptcha_service();
    
    // If reCAPTCHA is not enabled, skip verification
    if !recaptcha_service.is_enabled() {
        return Ok(());
    }
    
    // If reCAPTCHA is enabled but no token provided, return error
    let token = recaptcha_token.ok_or_else(|| {
        AppError::BadRequest("reCAPTCHA verification required but no token provided".to_string())
    })?;
    
    // Verify the token
    recaptcha_service
        .verify_token(token, client_ip)
        .await
        .map_err(|e| {
            tracing::warn!("reCAPTCHA verification failed: {}", e);
            AppError::BadRequest("reCAPTCHA verification failed. Please try again".to_string())
        })?;
    
    Ok(())
}

// Helper function to extract client IP from connection info
fn extract_client_ip(connect_info: Option<ConnectInfo<SocketAddr>>) -> Option<String> {
    connect_info.map(|ConnectInfo(addr)| addr.ip().to_string())
}

pub async fn get_video_info(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<VideoRequest>,
) -> Result<Json<VideoInfo>, AppError> {
    tracing::info!("Getting video info for URL: {} from IP: {}", request.url, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        request.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let video_info = service.get_video_info(&request.url).await?;
    
    Ok(Json(video_info))
}

// DEPRECATED: Legacy download endpoint - redirects to streaming for better performance
pub async fn download_video(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<DownloadRequest>,
) -> Result<Response, AppError> {
    tracing::warn!("Legacy download endpoint used, redirecting to streaming approach");
    tracing::info!("Streaming video from URL: {} with format_id: {} from IP: {}", 
                   request.url, request.format_id, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        request.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let (video_stream, filename) = service.stream_video(&request.url, &request.format_id).await?;
    
    // Create the streaming response with proper headers
    let stream = video_stream.map_err(|e| {
        tracing::error!("Stream error: {}", e);
        e
    });
    
    let body = Body::from_stream(stream);
    
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "video/mp4")
        .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        .header("Cache-Control", "no-cache")
        .header("Transfer-Encoding", "chunked")
        .body(body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build response: {}", e)))?;
    
    Ok(response)
}

// NEW STREAMING ENDPOINT
pub async fn stream_video_download(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<StreamDownloadQuery>,
) -> Result<Response, AppError> {
    tracing::info!("Streaming video from URL: {} with format_id: {} from IP: {}", 
                   params.url, params.format_id, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        params.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let (video_stream, filename) = service.stream_video(&params.url, &params.format_id).await?;
    
    // Create the streaming response with proper headers
    let stream = video_stream.map_err(|e| {
        tracing::error!("Stream error: {}", e);
        e
    });
    
    let body = Body::from_stream(stream);
    
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "video/mp4")
        .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        .header("Cache-Control", "no-cache")
        .header("Transfer-Encoding", "chunked")
        .body(body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build response: {}", e)))?;
    
    Ok(response)
}

/// NEW: Stream audio-only download (MP3) from TikTok video
pub async fn stream_audio_download(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<AudioStreamQuery>,
) -> Result<Response, AppError> {
    tracing::info!("Starting audio-only stream from URL: {} from IP: {}", params.url, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        params.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let (audio_stream, filename) = service.stream_audio(&params.url).await?;
    
    // Create the streaming response with proper MP3 headers
    let stream = audio_stream.map_err(|e| {
        tracing::error!("Audio stream error: {}", e);
        e
    });
    
    let body = Body::from_stream(stream);
    
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "audio/mpeg")
        .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        .header("Cache-Control", "no-cache")
        .header("Transfer-Encoding", "chunked")
        .body(body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build audio response: {}", e)))?;
    
    Ok(response)
}

// Enhanced error handling with different error types
#[derive(Debug)]
pub enum AppError {
    Internal(anyhow::Error),
    BadRequest(String),
    Unauthorized(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match self {
            AppError::Internal(err) => {
                tracing::error!("Internal error: {:?}", err);
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", format!("An error occurred: {}", err))
            }
            AppError::BadRequest(msg) => {
                tracing::warn!("Bad request: {}", msg);
                (StatusCode::BAD_REQUEST, "bad_request", msg)
            }
            AppError::Unauthorized(msg) => {
                tracing::warn!("Unauthorized: {}", msg);
                (StatusCode::UNAUTHORIZED, "unauthorized", msg)
            }
        };
        
        let error_response = ApiError::new(error_type, &message, status.as_u16());
        (status, Json(error_response)).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

// Profile Download Handlers - Phase 1

/// Get TikTok profile information (video count, estimated size)
pub async fn get_profile_info(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ProfileDownloadRequest>,
) -> Result<Json<ProfileInfo>, AppError> {
    tracing::info!("Getting profile info for URL: {} from IP: {}", request.profile_url, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        request.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let profile_info = service.get_profile_info(&request.profile_url).await?;
    
    Ok(Json(profile_info))
}

/// Download entire TikTok profile as ZIP archive
pub async fn download_profile_zip(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ProfileDownloadRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    tracing::info!("Starting profile ZIP download for URL: {} from IP: {}", 
                   request.profile_url, addr.ip());
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        request.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let (zip_path, zip_filename, zip_size) = service.download_profile_as_zip(&request.profile_url).await?;
    
    // Convert to absolute path string for streaming
    let zip_full_path = zip_path.to_string_lossy().to_string();
    
    // Return success response with FULL PATH for streaming
    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "Profile ZIP created successfully",
        "filename": zip_filename,
        "size": zip_size,
        "zip_path": zip_full_path, // NEW: Full path for streaming
        "download_url": format!("/api/profile/stream?zip_path={}", zip_full_path.replace("/", "%2F").replace("\\", "%5C"))
    })))
}

/// Phase 2: Download selected videos from TikTok profile as ZIP archive
pub async fn download_selected_profile_videos(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<SelectiveProfileDownloadRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    tracing::info!(
        "Starting selective profile download for URL: {} ({} videos selected) from IP: {}", 
        request.profile_url, 
        request.selected_video_urls.len(),
        addr.ip()
    );
    
    // Verify reCAPTCHA if enabled
    verify_recaptcha_if_enabled(
        request.recaptcha_token.as_ref(),
        Some(addr.ip().to_string()),
    ).await?;
    
    let service = TikTokService::new().map_err(|e| AppError::Internal(e))?;
    let (zip_path, zip_filename, zip_size) = service
        .download_selected_videos_as_zip(&request.profile_url, &request.selected_video_urls)
        .await?;
    
    // Convert to absolute path string for streaming
    let zip_full_path = zip_path.to_string_lossy().to_string();
    
    // Return success response with FULL PATH for streaming
    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Selected {} videos ZIP created successfully", request.selected_video_urls.len()),
        "filename": zip_filename,
        "size": zip_size,
        "selected_count": request.selected_video_urls.len(),
        "zip_path": zip_full_path, // NEW: Full path for streaming
        "download_url": format!("/api/profile/stream?zip_path={}", zip_full_path.replace("/", "%2F").replace("\\", "%5C"))
    })))
}

/// Stream profile ZIP file download (no reCAPTCHA needed - user already verified for creation)
pub async fn stream_profile_zip(
    Query(params): Query<ProfileStreamQuery>,
) -> Result<Response, AppError> {
    tracing::info!("Streaming profile ZIP file from: {}", params.zip_path);
    
    let zip_path = PathBuf::from(&params.zip_path);
    
    // Check if file exists
    if !zip_path.exists() {
        tracing::error!("ZIP file not found: {:?}", zip_path);
        return Err(AppError::BadRequest(format!("ZIP file not found: {}", params.zip_path)));
    }
    
    // Get filename for download header
    let filename = zip_path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.zip");
    
    // Open the ZIP file for streaming
    let file = File::open(&zip_path).await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to open ZIP file: {}", e)))?;
        
    // Create streaming response
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/zip")
        .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
        .header("Cache-Control", "no-cache")
        .body(body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build ZIP stream response: {}", e)))?;
    
    // Schedule cleanup of the ZIP file after streaming is complete
    let cleanup_path = params.zip_path.clone();
    tokio::spawn(async move {
        // Wait a bit longer to ensure streaming is complete
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        
        // Clean up the ZIP file
        if let Ok(service) = TikTokService::new() {
            if let Err(e) = service.cleanup_zip_file_by_path(&cleanup_path).await {
                tracing::warn!("Failed to cleanup ZIP file {}: {}", cleanup_path, e);
            }
        }
    });
    
    Ok(response)
}
