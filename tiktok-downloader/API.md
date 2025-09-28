# API Documentation

This document describes the TikTok Downloader API endpoints.

## Base URL

```
http://localhost:3001/api
```

## Endpoints

### Health Check

Check if the backend service is running.

```http
GET /health
```

**Response:**
```json
{
  "status": "healthy",
  "service": "tiktok-downloader-backend",
  "version": "0.1.0"
}
```

### Get Video Information

Retrieve metadata about a TikTok video.

```http
POST /video/info
Content-Type: application/json

{
  "url": "https://www.tiktok.com/@username/video/1234567890"
}
```

**Request Body:**
- `url` (string, required): Valid TikTok video URL

**Response:**
```json
{
  "id": "1234567890",
  "title": "Video Title",
  "author": "username",
  "description": "Video description",
  "duration": 30,
  "view_count": 10000,
  "like_count": 500,
  "share_count": 100,
  "comment_count": 50,
  "thumbnail_url": "https://example.com/thumbnail.jpg",
  "video_url": "https://example.com/video.mp4",
  "original_url": "https://www.tiktok.com/@username/video/1234567890",
  "available_formats": [
    {
      "format_id": "http-1080",
      "label": "1080p (HD) - high",
      "quality": "1080p",
      "ext": "mp4",
      "filesize": 5242880,
      "height": 1080,
      "width": 1920
    },
    {
      "format_id": "http-720",
      "label": "720p (HD) - medium",
      "quality": "720p",
      "ext": "mp4",
      "filesize": 3145728,
      "height": 720,
      "width": 1280
    }
  ],
  "created_at": "2024-01-15T10:30:00Z"
}
```

**Error Response:**
```json
{
  "error": "invalid_url",
  "message": "Invalid TikTok URL provided",
  "code": 400
}
```

### Download Video

Download a TikTok video with a specific format.

```http
POST /video/download
Content-Type: application/json

{
  "url": "https://www.tiktok.com/@username/video/1234567890",
  "format_id": "http-1080"
}
```

**Request Body:**
- `url` (string, required): Valid TikTok video URL
- `format_id` (string, required): Format ID from the available_formats list returned by `/video/info`

**Response:**
```json
{
  "download_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "completed",
  "file_url": "/downloads/username_video_title_1234567890_HD.mp4",
  "filename": "username_video_title_1234567890_HD.mp4",
  "file_size": 5242880,
  "progress": 100
}
```

**Response Fields:**
- `download_id` (string): Unique identifier for the download
- `status` (string): Download status (`"pending"`, `"downloading"`, `"completed"`, `"failed"`)
- `file_url` (string, nullable): URL to download the file (available when status is `"completed"`)
- `filename` (string): Generated filename for the video
- `file_size` (number, nullable): File size in bytes
- `progress` (number): Download progress percentage (0-100)

## Error Handling

All API endpoints return appropriate HTTP status codes:

- `200 OK`: Request succeeded
- `400 Bad Request`: Invalid request parameters
- `429 Too Many Requests`: Rate limit exceeded
- `500 Internal Server Error`: Server error

Error responses include a JSON body with error details:

```json
{
  "error": "error_code",
  "message": "Human-readable error message",
  "code": 400
}
```

## Rate Limiting

The API implements rate limiting to prevent abuse:

- **Limit**: 10 requests per minute per IP address
- **Response**: HTTP 429 when limit exceeded
- **Headers**: Rate limit information in response headers (future enhancement)

## Supported TikTok URL Formats

The API accepts the following TikTok URL formats:

- `https://www.tiktok.com/@username/video/1234567890`
- `https://tiktok.com/@username/video/1234567890`
- `https://vm.tiktok.com/ZTdXXXXXX/`
- `https://www.tiktok.com/t/ZTdXXXXXX/`
- `https://m.tiktok.com/v/1234567890.html`

## Video Quality Options

**Dynamic Format Selection (New):**
The API now provides dynamic format selection based on what's actually available for each video. Instead of static "high/medium/low" options, you get real format options with specific details:

1. **Get available formats** from `/video/info` endpoint
2. **Choose a format_id** from the `available_formats` array
3. **Use that format_id** in the `/video/download` request

**Format Object Structure:**
```json
{
  "format_id": "http-1080",      // Use this ID for downloading
  "label": "1080p (HD) - high",   // User-friendly display name
  "quality": "1080p",             // Resolution shorthand
  "ext": "mp4",                   // File extension
  "filesize": 5242880,            // File size in bytes (if available)
  "height": 1080,                 // Video height in pixels
  "width": 1920                   // Video width in pixels
}
```

**Benefits:**
- ✅ **No more "format not available" errors**
- ✅ **Shows actual file sizes**
- ✅ **Displays real resolution options**
- ✅ **Works with any TikTok video format**

## File Download

When a video download is completed, the `file_url` field contains a path to download the file:

```javascript
// Frontend example
const response = await fetch('/api/video/download', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ url: videoUrl, quality: 'high' })
});

const result = await response.json();

if (result.status === 'completed' && result.file_url) {
  // Create download link
  const link = document.createElement('a');
  link.href = result.file_url;
  link.download = result.filename;
  link.click();
}
```

## Security Considerations

- All requests are logged for monitoring purposes
- Rate limiting prevents API abuse
- Input validation ensures only valid TikTok URLs are processed
- Temporary files are automatically cleaned up
- CORS is configured for web browser access

## Future Enhancements

- **Authentication**: User accounts and API keys
- **Batch Downloads**: Multiple videos in one request
- **Webhook Support**: Notifications when downloads complete
- **Advanced Rate Limiting**: Per-user limits with Redis
- **File Storage**: Cloud storage integration
- **Analytics**: Download statistics and usage metrics
