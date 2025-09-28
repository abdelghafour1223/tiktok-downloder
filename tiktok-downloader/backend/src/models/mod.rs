use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoRequest {
    pub url: String,
    pub recaptcha_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FormatOption {
    pub format_id: String,
    pub label: String,
    pub quality: String,
    pub ext: String,
    pub filesize: Option<u64>,
    pub height: Option<u32>,
    pub width: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub duration: Option<u32>,
    pub view_count: Option<u64>,
    pub like_count: Option<u64>,
    pub share_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub thumbnail_url: Option<String>,
    pub video_url: String,
    pub original_url: String,
    pub available_formats: Vec<FormatOption>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub format_id: String,
    pub recaptcha_token: Option<String>,
}

// Profile Download Models - Phase 1 & 2
#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileDownloadRequest {
    pub profile_url: String,
    pub recaptcha_token: Option<String>,
}

// Phase 2: Enhanced ProfileDownloadRequest for selective downloads
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectiveProfileDownloadRequest {
    pub profile_url: String,
    pub selected_video_urls: Vec<String>,
    pub recaptcha_token: Option<String>,
}

// Phase 2: Individual video data for profile
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProfileVideoInfo {
    pub url: String,
    pub id: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub duration: Option<f64>,
    pub view_count: Option<u64>,
    pub upload_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub username: String,
    pub display_name: Option<String>,
    pub video_count: Option<u64>,
    pub estimated_zip_size: Option<u64>,
    pub total_downloadable_videos: u32,
    pub videos: Vec<ProfileVideoInfo>, // Phase 2: Full video list
}

#[derive(Debug, Deserialize)]
pub struct StreamDownloadQuery {
    pub url: String,
    pub format_id: String,
    pub recaptcha_token: Option<String>,
}

// NEW: Audio-only download query (no format_id needed)
#[derive(Debug, Deserialize)]
pub struct AudioStreamQuery {
    pub url: String,
    pub recaptcha_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileStreamQuery {
    pub zip_path: String, // CHANGED: Now expects full path instead of just filename
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadResponse {
    pub download_id: Uuid,
    pub status: DownloadStatus,
    pub file_url: Option<String>,
    pub filename: String,
    pub file_size: Option<u64>,
    pub progress: u8,
}

// reCAPTCHA verification models
#[derive(Debug, Deserialize)]
pub struct RecaptchaVerifyRequest {
    pub secret: String,
    pub response: String,
    pub remoteip: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecaptchaVerifyResponse {
    pub success: bool,
    #[serde(rename = "challenge_ts")]
    pub challenge_ts: Option<String>,
    pub hostname: Option<String>,
    #[serde(rename = "error-codes")]
    pub error_codes: Option<Vec<String>>,
    pub score: Option<f64>,
    pub action: Option<String>,
}

// Keep the old VideoQuality for backward compatibility in internal code
#[derive(Debug, Serialize, Deserialize)]
pub enum VideoQuality {
    #[serde(rename = "high")]
    High,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "low")]
    Low,
}

impl Default for VideoQuality {
    fn default() -> Self {
        VideoQuality::High
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DownloadStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "downloading")]
    Downloading,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    pub message: String,
    pub code: u16,
}

impl ApiError {
    pub fn new(error: &str, message: &str, code: u16) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
            code,
        }
    }
}
