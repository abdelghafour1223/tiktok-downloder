use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::{info, warn};

pub async fn logging_middleware(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    
    // Log request
    info!(
        method = %method,
        uri = %uri,
        user_agent = ?headers.get("user-agent"),
        "Request started"
    );

    let response = next.run(request).await;
    let status = response.status();
    let duration = start.elapsed();

    // Log response
    if status.is_success() {
        info!(
            method = %method,
            uri = %uri,
            status = %status,
            duration = ?duration,
            "Request completed successfully"
        );
    } else {
        warn!(
            method = %method,
            uri = %uri,
            status = %status,
            duration = ?duration,
            "Request completed with error"
        );
    }

    response
}

pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    
    let headers = response.headers_mut();
    
    // Add security headers
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    headers.insert(
        "Strict-Transport-Security",
        "max-age=31536000; includeSubDomains".parse().unwrap(),
    );
    headers.insert(
        "Content-Security-Policy",
        "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"
            .parse()
            .unwrap(),
    );

    response
}

// Simple rate limiting based on IP (for demonstration - use Redis in production)
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    max_requests: u32,
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_seconds,
        }
    }

    pub fn check_rate_limit(&self, client_ip: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut requests = self.requests.lock().unwrap();
        let client_requests = requests.entry(client_ip.to_string()).or_insert_with(Vec::new);

        // Remove old requests outside the time window
        client_requests.retain(|&timestamp| now - timestamp < self.window_seconds);

        if client_requests.len() >= self.max_requests as usize {
            false
        } else {
            client_requests.push(now);
            true
        }
    }
}

pub async fn rate_limit_middleware(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|hv| hv.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("unknown").trim())
        .unwrap_or("unknown");

    // For demonstration, we'll create a simple rate limiter
    // In production, use a proper distributed rate limiter with Redis
    let rate_limiter = RateLimiter::new(10, 60); // 10 requests per minute

    if !rate_limiter.check_rate_limit(client_ip) {
        warn!(client_ip = %client_ip, "Rate limit exceeded");
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}
