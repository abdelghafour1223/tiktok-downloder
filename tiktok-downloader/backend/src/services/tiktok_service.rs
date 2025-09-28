use anyhow::{anyhow, Result};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::stream::Stream;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tokio::process::Command;
use uuid::Uuid;

// Profile download functionality
use std::path::{Path, PathBuf};
use tokio::fs;
use zip::write::FileOptions;
use zip::ZipWriter;
use std::io::Write;

use crate::models::*;
use crate::utils::url_validator::{is_valid_tiktok_url, is_valid_tiktok_profile_url, extract_tiktok_username};

// yt-dlp JSON response structures for profile videos
#[derive(Debug, Deserialize)]
struct YtDlpProfileEntry {
    id: String,
    title: Option<String>,
    url: String,
    thumbnail: Option<String>, // Keep this for backward compatibility
    thumbnails: Option<Vec<YtDlpThumbnail>>, // NEW: Handle thumbnails array
    duration: Option<f64>,
    view_count: Option<u64>,
    upload_date: Option<String>,
    webpage_url: Option<String>,
}

// NEW: Structure for individual thumbnail objects
#[derive(Debug, Deserialize)]
struct YtDlpThumbnail {
    id: Option<String>,
    url: String,
    height: Option<u32>,
    width: Option<u32>,
}

// Global counter for generating sequential filenames
static DOWNLOAD_COUNTER: AtomicU32 = AtomicU32::new(1);

// Stream wrapper for yt-dlp stdout
pub struct VideoStream {
    reader: tokio::process::ChildStdout,
    child: tokio::process::Child,
}

impl Stream for VideoStream {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check if child process is still running
        if let Ok(Some(exit_status)) = self.child.try_wait() {
            if !exit_status.success() {
                tracing::error!("yt-dlp process exited with error: {:?}", exit_status);
                return Poll::Ready(Some(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "yt-dlp process failed"
                ))));
            }
        }

        // Read data from stdout
        let mut buffer = vec![0u8; 8192]; // 8KB buffer
        let mut read_buf = ReadBuf::new(&mut buffer);
        
        match Pin::new(&mut self.reader).poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                let filled = read_buf.filled().len();
                if filled == 0 {
                    // EOF reached
                    tracing::info!("Video stream completed");
                    Poll::Ready(None)
                } else {
                    let data = read_buf.filled().to_vec();
                    Poll::Ready(Some(Ok(bytes::Bytes::from(data))))
                }
            }
            Poll::Ready(Err(e)) => {
                tracing::error!("Error reading from yt-dlp stdout: {}", e);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        // Ensure child process is killed when stream is dropped
        if let Err(e) = self.child.start_kill() {
            tracing::warn!("Failed to kill yt-dlp child process: {}", e);
        }
    }
}

// yt-dlp JSON response structures
#[derive(Debug, Deserialize)]
struct YtDlpVideoInfo {
    id: String,
    title: Option<String>,
    description: Option<String>,
    uploader: Option<String>,
    uploader_id: Option<String>,
    duration: Option<f64>,
    view_count: Option<u64>,
    like_count: Option<u64>,
    comment_count: Option<u64>,
    thumbnail: Option<String>, // Keep for backward compatibility
    thumbnails: Option<Vec<YtDlpThumbnail>>, // NEW: Handle thumbnails array
    webpage_url: String,
    upload_date: Option<String>,
    formats: Option<Vec<YtDlpFormat>>,
}

#[derive(Debug, Deserialize)]
struct YtDlpFormat {
    format_id: String,
    ext: String,
    quality: Option<f64>,
    height: Option<u32>,
    width: Option<u32>,
    filesize: Option<u64>,
    url: Option<String>,
    vcodec: Option<String>,
    acodec: Option<String>,
    format_note: Option<String>,
}

pub struct TikTokService {
    temp_dir: TempDir,
    downloads_dir: PathBuf, // NEW: Permanent downloads directory for ZIPs
}

impl TikTokService {
    /// Get the path to the temporary directory
    pub fn temp_dir_path(&self) -> &Path {
        self.temp_dir.path()
    }
    
    /// Get the path to the permanent downloads directory
    pub fn downloads_dir_path(&self) -> &Path {
        &self.downloads_dir
    }
    
    /// Extract the best thumbnail URL from yt-dlp thumbnails array
    fn extract_best_thumbnail_url(thumbnails: &Option<Vec<YtDlpThumbnail>>, fallback: &Option<String>) -> Option<String> {
        // First, try to get thumbnail from the thumbnails array
        if let Some(thumbnails_array) = thumbnails {
            if !thumbnails_array.is_empty() {
                // Strategy 1: Look for "cover" thumbnail first (best quality for TikTok)
                if let Some(cover_thumbnail) = thumbnails_array.iter().find(|t| {
                    t.id.as_ref().map(|id| id.contains("cover")).unwrap_or(false)
                }) {
                    tracing::debug!("Found cover thumbnail: {}", cover_thumbnail.url);
                    return Some(cover_thumbnail.url.clone());
                }
                
                // Strategy 2: Look for highest resolution thumbnail
                if let Some(best_thumbnail) = thumbnails_array.iter().max_by_key(|t| {
                    t.height.unwrap_or(0) * t.width.unwrap_or(0)
                }) {
                    tracing::debug!("Found high-res thumbnail: {}", best_thumbnail.url);
                    return Some(best_thumbnail.url.clone());
                }
                
                // Strategy 3: Just take the first thumbnail
                if let Some(first_thumbnail) = thumbnails_array.first() {
                    tracing::debug!("Using first thumbnail: {}", first_thumbnail.url);
                    return Some(first_thumbnail.url.clone());
                }
            }
        }
        
        // Fallback to the old single thumbnail field
        if let Some(single_thumbnail) = fallback {
            tracing::debug!("Using fallback thumbnail: {}", single_thumbnail);
            return Some(single_thumbnail.clone());
        }
        
        tracing::debug!("No thumbnail found");
        None
    }
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        tracing::info!("Created temporary directory: {:?}", temp_dir.path());
        
        // Use a default downloads directory - this should be passed from config
        let downloads_dir = PathBuf::from("./downloads");
        Self::new_with_downloads_dir(downloads_dir)
    }
    
    /// Create a new TikTokService with a specific downloads directory
    pub fn new_with_downloads_dir(downloads_dir: PathBuf) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        tracing::info!("Created temporary directory: {:?}", temp_dir.path());
        
        // Ensure downloads directory exists
        if !downloads_dir.exists() {
            std::fs::create_dir_all(&downloads_dir)
                .map_err(|e| anyhow!("Failed to create downloads directory {:?}: {}", downloads_dir, e))?;
            tracing::info!("Created downloads directory: {:?}", downloads_dir);
        }
        
        Ok(Self { 
            temp_dir, 
            downloads_dir 
        })
    }

    /// Check if yt-dlp is installed and accessible
    pub async fn check_ytdlp_availability(&self) -> Result<()> {
        if which::which("yt-dlp").is_err() {
            return Err(anyhow!(
                "yt-dlp is not installed or not found in PATH. Please install it from: https://github.com/yt-dlp/yt-dlp"
            ));
        }

        let output = Command::new("yt-dlp")
            .arg("--version")
            .output()
            .await?;

        if !output.status.success() {
            return Err(anyhow!("yt-dlp is installed but not working properly"));
        }

        let version = String::from_utf8_lossy(&output.stdout);
        tracing::info!("yt-dlp version: {}", version.trim());
        
        Ok(())
    }

    pub async fn get_video_info(&self, url: &str) -> Result<VideoInfo> {
        if !is_valid_tiktok_url(url) {
            return Err(anyhow!("Invalid TikTok URL provided"));
        }

        self.check_ytdlp_availability().await?;
        tracing::info!("Extracting video info from URL: {}", url);

        let ytdlp_info = self.extract_video_metadata(url).await?;
        let video_info = self.convert_ytdlp_to_video_info(ytdlp_info, url).await?;
        
        Ok(video_info)
    }

    /// DEPRECATED: Use stream_video instead for direct streaming downloads
    /// This method now redirects to streaming to eliminate server disk usage
    pub async fn download_video(&self, url: &str, format_id: &str) -> Result<(VideoStream, String)> {
        tracing::warn!("download_video is deprecated, redirecting to stream_video for better performance");
        self.stream_video(url, format_id).await
    }

    /// Stream video directly from yt-dlp stdout - NEW STREAMING METHOD
    pub async fn stream_video(&self, url: &str, format_id: &str) -> Result<(VideoStream, String)> {
        if !is_valid_tiktok_url(url) {
            return Err(anyhow!("Invalid TikTok URL provided"));
        }

        self.check_ytdlp_availability().await?;
        tracing::info!("Starting video stream from URL: {} with format_id: {}", url, format_id);

        // Get video info for filename generation
        let video_info = self.get_video_info(url).await?;
        
        // Verify format_id exists in available formats
        if !video_info.available_formats.iter().any(|f| f.format_id == format_id) {
            return Err(anyhow!("Invalid format_id: {}. Available formats: {:?}", 
                format_id, 
                video_info.available_formats.iter().map(|f| &f.format_id).collect::<Vec<_>>()
            ));
        }

        // Generate a simple filename for the download
        let counter = DOWNLOAD_COUNTER.fetch_add(1, Ordering::SeqCst);
        let filename = format!("topclipdowload{}.mp4", counter);
        
        tracing::info!("Streaming video with filename: {}", filename);

        // Start yt-dlp process with stdout streaming - NO FFmpeg processing for maximum compatibility
        let mut cmd = Command::new("yt-dlp");
        cmd.args(&[
            "--no-warnings",
            "--no-post-overwrites",    // Skip post-processing
            "--no-embed-subs",        // Skip subtitle embedding
            "--no-embed-chapters",    // Skip chapter embedding  
            "--no-embed-info-json",   // Skip metadata embedding
            "-f", format_id,
            "-o", "-", // CRITICAL: Stream to stdout instead of file
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        tracing::debug!("Executing streaming yt-dlp command: {:?}", cmd);

        let mut child = cmd.spawn()?;
        
        // Take stdout from the child process
        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow!("Failed to capture yt-dlp stdout"))?;

        // Create a stream wrapper
        let stream = VideoStream {
            reader: stdout,
            child,
        };

        Ok((stream, filename))
    }
    
    /// Stream audio-only from TikTok video as MP3
    pub async fn stream_audio(&self, url: &str) -> Result<(VideoStream, String)> {
        if !is_valid_tiktok_url(url) {
            return Err(anyhow!("Invalid TikTok URL provided"));
        }

        self.check_ytdlp_availability().await?;
        tracing::info!("Starting audio-only stream from URL: {}", url);

        // Generate a simple filename for the audio download
        let counter = DOWNLOAD_COUNTER.fetch_add(1, Ordering::SeqCst);
        let filename = format!("tiktok_audio_{}.mp3", counter);
        
        tracing::info!("Streaming audio with filename: {}", filename);

        // Start yt-dlp process with audio extraction and stdout streaming
        let mut cmd = Command::new("yt-dlp");
        cmd.args(&[
            "-x", // Extract audio
            "--audio-format", "mp3", // Convert to MP3
            "--no-warnings",
            "--no-post-overwrites",
            "-o", "-", // CRITICAL: Stream to stdout instead of file
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        tracing::debug!("Executing audio streaming yt-dlp command: {:?}", cmd);

        let mut child = cmd.spawn()?;
        
        // Take stdout from the child process
        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow!("Failed to capture yt-dlp stdout for audio"))?;

        // Create a stream wrapper
        let stream = VideoStream {
            reader: stdout,
            child,
        };

        Ok((stream, filename))
    }

    // Profile Download Methods - Phase 1 & 2
    
    /// Get detailed TikTok profile information including full video list (Phase 2)
    pub async fn get_profile_info(&self, profile_url: &str) -> Result<ProfileInfo> {
        if !is_valid_tiktok_profile_url(profile_url) {
            return Err(anyhow!("Invalid TikTok profile URL provided"));
        }

        self.check_ytdlp_availability().await?;
        
        let username = extract_tiktok_username(profile_url)
            .ok_or_else(|| anyhow!("Failed to extract username from profile URL"))?;
            
        tracing::info!("Getting detailed profile info for: @{}", username);

        // Phase 2: Get detailed video list with metadata
        let videos = self.get_profile_video_list(profile_url).await?;
        let video_count = videos.len() as u32;
        
        // Create profile info with detailed video list
        let profile_info = ProfileInfo {
            username: username.clone(),
            display_name: Some(format!("@{}", username)),
            video_count: Some(video_count as u64),
            estimated_zip_size: Some((video_count as u64) * 5_000_000), // Rough estimate: 5MB per video
            total_downloadable_videos: video_count,
            videos, // Phase 2: Include full video list
        };

        Ok(profile_info)
    }
    
    /// Phase 2: Get detailed list of all videos in a profile
    async fn get_profile_video_list(&self, profile_url: &str) -> Result<Vec<ProfileVideoInfo>> {
        tracing::info!("Getting detailed video list for profile: {}", profile_url);

        let output = Command::new("yt-dlp")
            .args(&[
                "--dump-json",
                "--flat-playlist",
                "--no-warnings",
                "--no-download", // Don't actually download videos, just get metadata
                profile_url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            tracing::error!("yt-dlp profile video list error: {}", error_msg);
            return Err(anyhow!("Failed to get profile video list: {}", error_msg));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut videos = Vec::new();
        
        // Parse each line as a separate JSON object (yt-dlp outputs one JSON per line)
        for (index, line) in output_str.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            
            match serde_json::from_str::<YtDlpProfileEntry>(line) {
                Ok(entry) => {
                    // Use the new smart thumbnail extraction method
                    let thumbnail_url = Self::extract_best_thumbnail_url(&entry.thumbnails, &entry.thumbnail);
                    
                    tracing::debug!(
                        "Video {}: thumbnails_array={}, single_thumbnail={}, final_url={}",
                        entry.id,
                        entry.thumbnails.as_ref().map(|t| t.len()).unwrap_or(0),
                        entry.thumbnail.is_some(),
                        thumbnail_url.is_some()
                    );
                    
                    let video_info = ProfileVideoInfo {
                        url: entry.webpage_url.unwrap_or(entry.url),
                        id: entry.id.clone(),
                        title: entry.title.unwrap_or_else(|| format!("TikTok Video #{}", index + 1)),
                        thumbnail_url, // Now using the extracted thumbnail URL
                        duration: entry.duration,
                        view_count: entry.view_count,
                        upload_date: entry.upload_date,
                    };
                    videos.push(video_info);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse video entry JSON: {}", e);
                    continue;
                }
            }
        }
        
        if videos.is_empty() {
            tracing::warn!("No videos found in profile, trying alternative method...");
            // Try alternative method without --flat-playlist
            return self.get_profile_video_list_alternative(profile_url).await;
        }
        
        // Log thumbnail extraction success rate
        let thumbnail_count = videos.iter().filter(|v| v.thumbnail_url.is_some()).count();
        tracing::info!(
            "Found {} videos in profile, {} have thumbnails ({:.1}% success rate)",
            videos.len(),
            thumbnail_count,
            (thumbnail_count as f64 / videos.len() as f64) * 100.0
        );
        
        Ok(videos)
    }
    
    /// Alternative method to get video list with better thumbnail support
    async fn get_profile_video_list_alternative(&self, profile_url: &str) -> Result<Vec<ProfileVideoInfo>> {
        tracing::info!("Trying alternative method to get video list with thumbnails");

        let output = Command::new("yt-dlp")
            .args(&[
                "--dump-json",
                "--no-download",
                "--no-warnings",
                "--playlist-end", "50", // Limit to first 50 videos for better performance
                profile_url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Alternative method error: {}", error_msg);
            return Ok(vec![]); // Return empty vec instead of error
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut videos = Vec::new();
        
        // Parse each line as a separate JSON object
        for (index, line) in output_str.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            
            // Try to parse as full video info first
            if let Ok(full_info) = serde_json::from_str::<YtDlpVideoInfo>(line) {
                // Use the same smart thumbnail extraction for full video info
                let thumbnail_url = Self::extract_best_thumbnail_url(&full_info.thumbnails, &full_info.thumbnail);
                
                tracing::debug!(
                    "Alternative method - Video {}: thumbnails_array={}, single_thumbnail={}, final_url={}",
                    full_info.id,
                    full_info.thumbnails.as_ref().map(|t| t.len()).unwrap_or(0),
                    full_info.thumbnail.is_some(),
                    thumbnail_url.is_some()
                );
                
                let video_info = ProfileVideoInfo {
                    url: full_info.webpage_url,
                    id: full_info.id,
                    title: full_info.title.unwrap_or_else(|| format!("TikTok Video #{}", index + 1)),
                    thumbnail_url, // Now using the extracted thumbnail URL
                    duration: full_info.duration.map(|d| d as f64),
                    view_count: full_info.view_count,
                    upload_date: full_info.upload_date,
                };
                videos.push(video_info);
                
                // Limit to prevent too many API calls
                if videos.len() >= 50 {
                    break;
                }
            }
        }
        
        // Log thumbnail extraction success rate for alternative method
        let thumbnail_count = videos.iter().filter(|v| v.thumbnail_url.is_some()).count();
        tracing::info!(
            "Alternative method found {} videos, {} have thumbnails ({:.1}% success rate)",
            videos.len(),
            thumbnail_count,
            if videos.is_empty() { 0.0 } else { (thumbnail_count as f64 / videos.len() as f64) * 100.0 }
        );
        
        Ok(videos)
    }
    
    /// Count total videos in a profile
    async fn count_profile_videos(&self, profile_url: &str) -> Result<u32> {
        tracing::info!("Counting videos in profile: {}", profile_url);

        let output = Command::new("yt-dlp")
            .args(&[
                "--flat-playlist",
                "--no-warnings",
                "--print", "%(title)s",
                profile_url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Could not count videos precisely: {}", error_msg);
            return Ok(10); // Default estimate
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let video_count = output_str.lines().count() as u32;
        
        tracing::info!("Found {} videos in profile", video_count);
        Ok(video_count)
    }
    
    /// Download entire profile as ZIP
    pub async fn download_profile_as_zip(&self, profile_url: &str) -> Result<(PathBuf, String, u64)> {
        if !is_valid_tiktok_profile_url(profile_url) {
            return Err(anyhow!("Invalid TikTok profile URL provided"));
        }

        self.check_ytdlp_availability().await?;
        
        let username = extract_tiktok_username(profile_url)
            .ok_or_else(|| anyhow!("Failed to extract username from profile URL"))?;
            
        tracing::info!("Starting profile download for: @{}", username);

        // Create unique temporary subdirectory for this download session
        let session_id = Uuid::new_v4();
        let session_dir = self.temp_dir.path().join(format!("profile_{}_{}", username, session_id));
        fs::create_dir_all(&session_dir).await?;
        
        tracing::info!("Created session directory: {:?}", session_dir);

        // Download all videos from profile
        let video_files = self.download_all_profile_videos(profile_url, &session_dir).await?;
        
        if video_files.is_empty() {
            return Err(anyhow!("No videos were downloaded from the profile"));
        }

        tracing::info!("Downloaded {} videos, creating ZIP archive", video_files.len());

        // Create ZIP archive in PERMANENT downloads directory (not temp)
        let zip_filename = format!("tiktok_profile_{}.zip", username);
        let zip_path = self.downloads_dir.join(&zip_filename); // CHANGED: Use downloads_dir
        let zip_size = self.create_zip_archive(&video_files, &zip_path).await?;

        // Clean up individual video files (keep only the ZIP)
        self.cleanup_video_files(&video_files).await?;
        fs::remove_dir_all(&session_dir).await.unwrap_or_else(|e| {
            tracing::warn!("Failed to remove session directory: {}", e);
        });

        tracing::info!("ZIP archive created: {:?} ({} bytes)", zip_path, zip_size);
        Ok((zip_path, zip_filename, zip_size))
    }
    
    /// Phase 2: Download selected videos from profile as ZIP
    pub async fn download_selected_videos_as_zip(
        &self,
        profile_url: &str,
        selected_video_urls: &[String],
    ) -> Result<(PathBuf, String, u64)> {
        if !is_valid_tiktok_profile_url(profile_url) {
            return Err(anyhow!("Invalid TikTok profile URL provided"));
        }

        if selected_video_urls.is_empty() {
            return Err(anyhow!("No videos selected for download"));
        }

        self.check_ytdlp_availability().await?;
        
        let username = extract_tiktok_username(profile_url)
            .ok_or_else(|| anyhow!("Failed to extract username from profile URL"))?;
            
        tracing::info!(
            "Starting selective download for: @{} ({} videos selected)", 
            username, 
            selected_video_urls.len()
        );

        // Create unique temporary subdirectory for this download session
        let session_id = Uuid::new_v4();
        let session_dir = self.temp_dir.path().join(format!("selective_{}_{}", username, session_id));
        fs::create_dir_all(&session_dir).await?;
        
        tracing::info!("Created session directory: {:?}", session_dir);

        // Download selected videos
        let video_files = self.download_selected_videos(selected_video_urls, &session_dir).await?;
        
        if video_files.is_empty() {
            return Err(anyhow!("No videos were downloaded from the selection"));
        }

        tracing::info!("Downloaded {} selected videos, creating ZIP archive", video_files.len());

        // Create ZIP archive in PERMANENT downloads directory (not temp)
        let zip_filename = format!("tiktok_selected_{}_{}_videos.zip", username, video_files.len());
        let zip_path = self.downloads_dir.join(&zip_filename); // CHANGED: Use downloads_dir
        let zip_size = self.create_zip_archive(&video_files, &zip_path).await?;

        // Clean up individual video files (keep only the ZIP)
        self.cleanup_video_files(&video_files).await?;
        fs::remove_dir_all(&session_dir).await.unwrap_or_else(|e| {
            tracing::warn!("Failed to remove session directory: {}", e);
        });

        tracing::info!("ZIP archive created: {:?} ({} bytes)", zip_path, zip_size);
        Ok((zip_path, zip_filename, zip_size))
    }
    
    /// Download all videos from a TikTok profile
    async fn download_all_profile_videos(&self, profile_url: &str, output_dir: &Path) -> Result<Vec<PathBuf>> {
        tracing::info!("Downloading all videos from profile to: {:?}", output_dir);

        // Build yt-dlp command for downloading all videos
        let mut cmd = Command::new("yt-dlp");
        cmd.args(&[
            "--no-warnings",
            "--no-post-overwrites",
            "--format", "best[ext=mp4]", // Prefer MP4 format
            "--output", &format!("{}/%(uploader)s_%(title)s_%(id)s.%(ext)s", output_dir.display()),
            profile_url,
        ]);

        tracing::debug!("Executing profile download command: {:?}", cmd);

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            tracing::error!("yt-dlp profile download error: {}", error_msg);
            return Err(anyhow!("Failed to download profile videos: {}", error_msg));
        }

        // Collect all downloaded video files
        let mut video_files = Vec::new();
        let mut entries = fs::read_dir(output_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "mp4" || ext == "webm" || ext == "mkv" {
                        video_files.push(path);
                    }
                }
            }
        }

        tracing::info!("Downloaded {} video files", video_files.len());
        Ok(video_files)
    }
    
    /// Phase 2: Download specific videos from URLs
    async fn download_selected_videos(&self, video_urls: &[String], output_dir: &Path) -> Result<Vec<PathBuf>> {
        tracing::info!("Downloading {} selected videos to: {:?}", video_urls.len(), output_dir);
        
        let mut video_files = Vec::new();
        
        // Download each video individually
        for (index, video_url) in video_urls.iter().enumerate() {
            tracing::info!("Downloading video {} of {}: {}", index + 1, video_urls.len(), video_url);
            
            // Build yt-dlp command for individual video
            let mut cmd = Command::new("yt-dlp");
            cmd.args(&[
                "--no-warnings",
                "--no-post-overwrites",
                "--format", "best[ext=mp4]", // Prefer MP4 format
                "--output", &format!("{}/%(uploader)s_%(title)s_%(id)s.%(ext)s", output_dir.display()),
                video_url,
            ]);

            tracing::debug!("Executing video download command: {:?}", cmd);

            let output = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await?;

            if !output.status.success() {
                let error_msg = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("Failed to download video {}: {}", video_url, error_msg);
                continue; // Skip failed downloads but continue with others
            }
        }
        
        // Collect all downloaded video files
        let mut entries = fs::read_dir(output_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "mp4" || ext == "webm" || ext == "mkv" {
                        video_files.push(path);
                    }
                }
            }
        }

        tracing::info!("Successfully downloaded {} video files", video_files.len());
        Ok(video_files)
    }
    
    /// Create ZIP archive from video files
    async fn create_zip_archive(&self, video_files: &[PathBuf], zip_path: &Path) -> Result<u64> {
        let zip_file = std::fs::File::create(zip_path)?;
        let mut zip = ZipWriter::new(zip_file);
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for video_file in video_files {
            let file_name = video_file.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("Invalid filename in video file"))?;
            
            tracing::debug!("Adding to ZIP: {}", file_name);
            zip.start_file(file_name, options)?;
            
            let file_data = fs::read(video_file).await?;
            zip.write_all(&file_data)?;
        }

        zip.finish()?;
        
        // Get ZIP file size
        let metadata = fs::metadata(zip_path).await?;
        Ok(metadata.len())
    }
    
    /// Clean up individual video files after ZIP creation
    async fn cleanup_video_files(&self, video_files: &[PathBuf]) -> Result<()> {
        for video_file in video_files {
            if let Err(e) = fs::remove_file(video_file).await {
                tracing::warn!("Failed to remove video file {:?}: {}", video_file, e);
            }
        }
        Ok(())
    }
    
    /// Clean up ZIP file after streaming
    pub async fn cleanup_zip_file(&self, zip_path: &Path) -> Result<()> {
        if let Err(e) = fs::remove_file(zip_path).await {
            tracing::warn!("Failed to remove ZIP file {:?}: {}", zip_path, e);
        } else {
            tracing::info!("Successfully cleaned up ZIP file: {:?}", zip_path);
        }
        Ok(())
    }
    
    /// Clean up ZIP file from downloads directory by full path
    pub async fn cleanup_zip_file_by_path(&self, zip_full_path: &str) -> Result<()> {
        let zip_path = PathBuf::from(zip_full_path);
        self.cleanup_zip_file(&zip_path).await
    }

    async fn extract_video_metadata(&self, url: &str) -> Result<YtDlpVideoInfo> {
        tracing::debug!("Calling yt-dlp to extract metadata for: {}", url);

        let output = Command::new("yt-dlp")
            .args(&[
                "--dump-json",
                "--no-download",
                "--no-warnings",
                url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            tracing::error!("yt-dlp error: {}", error_msg);
            return Err(anyhow!("Failed to extract video metadata: {}", error_msg));
        }

        let json_output = String::from_utf8(output.stdout)?;
        tracing::debug!("yt-dlp JSON output length: {} characters", json_output.len());

        let video_info: YtDlpVideoInfo = serde_json::from_str(&json_output)
            .map_err(|e| anyhow!("Failed to parse yt-dlp JSON output: {}", e))?;

        Ok(video_info)
    }

    async fn convert_ytdlp_to_video_info(
        &self,
        ytdlp_info: YtDlpVideoInfo,
        original_url: &str,
    ) -> Result<VideoInfo> {
        // Parse upload date if available
        let created_at = if let Some(upload_date) = &ytdlp_info.upload_date {
            if upload_date.len() == 8 {
                let year: i32 = upload_date[0..4].parse().unwrap_or(2024);
                let month: u32 = upload_date[4..6].parse().unwrap_or(1);
                let day: u32 = upload_date[6..8].parse().unwrap_or(1);
                
                chrono::NaiveDate::from_ymd_opt(year, month, day)
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|datetime| DateTime::<Utc>::from_naive_utc_and_offset(datetime, Utc))
                    .unwrap_or_else(Utc::now)
            } else {
                Utc::now()
            }
        } else {
            Utc::now()
        };

        // Parse available formats from yt-dlp data
        let available_formats = self.parse_available_formats(&ytdlp_info.formats)?;

        // Extract the best video URL from formats (for display purposes)
        let video_url = ytdlp_info
            .formats
            .as_ref()
            .and_then(|formats| {
                formats
                    .iter()
                    .filter(|f| f.ext == "mp4" && f.url.is_some())
                    .max_by_key(|f| f.quality.unwrap_or(0.0) as i32)
                    .and_then(|f| f.url.clone())
            })
            .unwrap_or_else(|| "".to_string());

        // Use the same smart thumbnail extraction logic for consistency
        let thumbnail_url = Self::extract_best_thumbnail_url(&ytdlp_info.thumbnails, &ytdlp_info.thumbnail);

        let video_info = VideoInfo {
            id: ytdlp_info.id,
            title: ytdlp_info.title.unwrap_or_else(|| "Untitled".to_string()),
            author: ytdlp_info.uploader_id.unwrap_or_else(|| "unknown".to_string()),
            description: ytdlp_info.description.unwrap_or_else(|| "".to_string()),
            duration: ytdlp_info.duration.map(|d| d as u32),
            view_count: ytdlp_info.view_count,
            like_count: ytdlp_info.like_count,
            share_count: None,
            comment_count: ytdlp_info.comment_count,
            thumbnail_url, // Now using the extracted thumbnail URL
            video_url,
            original_url: original_url.to_string(),
            available_formats,
            created_at,
        };

        Ok(video_info)
    }

    fn parse_available_formats(&self, formats: &Option<Vec<YtDlpFormat>>) -> Result<Vec<FormatOption>> {
        let formats = match formats {
            Some(f) => f,
            None => return Ok(vec![]),
        };

        let mut available_formats = Vec::new();
        
        // Filter and process formats
        for format in formats {
            // Only include video formats with reasonable quality
            if format.ext != "mp4" || format.height.is_none() {
                continue;
            }
            
            // Skip audio-only or very low quality formats
            if let Some(vcodec) = &format.vcodec {
                if vcodec == "none" {
                    continue;
                }
            }
            
            let height = format.height.unwrap_or(0);
            if height < 240 {
                continue; // Skip very low quality
            }

            // Create user-friendly label
            let quality_label = if height >= 1080 {
                "1080p (HD)"
            } else if height >= 720 {
                "720p (HD)"
            } else if height >= 480 {
                "480p"
            } else {
                "360p"
            };

            let label = if let Some(note) = &format.format_note {
                format!("{} - {}", quality_label, note)
            } else {
                quality_label.to_string()
            };

            available_formats.push(FormatOption {
                format_id: format.format_id.clone(),
                label,
                quality: format!("{}p", height),
                ext: format.ext.clone(),
                filesize: format.filesize,
                height: format.height,
                width: format.width,
            });
        }

        // Sort by quality (height) descending
        available_formats.sort_by(|a, b| {
            b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0))
        });

        // Remove duplicates based on height
        available_formats.dedup_by(|a, b| a.height == b.height);

        // Limit to top 5 formats to avoid overwhelming the user
        available_formats.truncate(5);

        if available_formats.is_empty() {
            tracing::warn!("No suitable video formats found");
            // Provide fallback format
            available_formats.push(FormatOption {
                format_id: "best".to_string(),
                label: "Best Available".to_string(),
                quality: "auto".to_string(),
                ext: "mp4".to_string(),
                filesize: None,
                height: None,
                width: None,
            });
        }

        tracing::info!("Found {} available formats", available_formats.len());
        for fmt in &available_formats {
            tracing::debug!("Format: {} - {} ({})", fmt.format_id, fmt.label, fmt.quality);
        }

        Ok(available_formats)
    }

    fn sanitize_filename(&self, input: &str) -> String {
        // Remove or replace invalid filename characters
        input
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
            .collect::<String>()
            .replace(' ', "_")
            .chars()
            .take(30) // Limit length
            .collect()
    }

    /// Clean up temporary files
    pub async fn cleanup(&self) -> Result<()> {
        tracing::info!("Cleaning up temporary files in: {:?}", self.temp_dir.path());
        Ok(())
    }
}

impl Drop for TikTokService {
    fn drop(&mut self) {
        tracing::debug!("TikTokService dropped, temporary directory will be cleaned up");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ytdlp_availability() {
        let service = TikTokService::new().unwrap();
        
        match service.check_ytdlp_availability().await {
            Ok(_) => println!("yt-dlp is available"),
            Err(e) => println!("yt-dlp is not available: {}", e),
        }
    }

    #[test]
    fn test_extract_best_thumbnail_url() {
        // Test with thumbnails array
        let thumbnails = vec![
            YtDlpThumbnail {
                id: Some("dynamic".to_string()),
                url: "https://example.com/dynamic.jpg".to_string(),
                height: Some(720),
                width: Some(720),
            },
            YtDlpThumbnail {
                id: Some("cover".to_string()),
                url: "https://example.com/cover.jpg".to_string(),
                height: Some(1080),
                width: Some(1080),
            },
        ];
        
        // Should prefer "cover" thumbnail
        let result = TikTokService::extract_best_thumbnail_url(&Some(thumbnails), &None);
        assert_eq!(result, Some("https://example.com/cover.jpg".to_string()));
        
        // Test with fallback
        let fallback = Some("https://example.com/fallback.jpg".to_string());
        let result = TikTokService::extract_best_thumbnail_url(&None, &fallback);
        assert_eq!(result, Some("https://example.com/fallback.jpg".to_string()));
        
        // Test with no thumbnails
        let result = TikTokService::extract_best_thumbnail_url(&None, &None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_available_formats() {
        let service = TikTokService::new().unwrap();
        
        let test_formats = vec![
            YtDlpFormat {
                format_id: "test1".to_string(),
                ext: "mp4".to_string(),
                quality: Some(1080.0),
                height: Some(1080),
                width: Some(1920),
                filesize: Some(5000000),
                url: Some("test_url".to_string()),
                vcodec: Some("h264".to_string()),
                acodec: Some("aac".to_string()),
                format_note: Some("high".to_string()),
            },
            YtDlpFormat {
                format_id: "test2".to_string(),
                ext: "mp4".to_string(),
                quality: Some(720.0),
                height: Some(720),
                width: Some(1280),
                filesize: Some(3000000),
                url: Some("test_url2".to_string()),
                vcodec: Some("h264".to_string()),
                acodec: Some("aac".to_string()),
                format_note: Some("medium".to_string()),
            },
        ];

        let result = service.parse_available_formats(&Some(test_formats)).unwrap();
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].quality, "1080p"); // Should be sorted by quality desc
        assert_eq!(result[1].quality, "720p");
    }
}