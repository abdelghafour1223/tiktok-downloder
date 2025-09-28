use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub port: u16,
    pub host: String,
    pub max_file_size: u64,
    pub temp_dir: String,
    pub cleanup_interval: u64,
    pub rate_limit_requests: u32,
    pub rate_limit_window: u64,
    pub cors_origins: Vec<String>,
    pub recaptcha_secret_key: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: 3001,
            host: "127.0.0.1".to_string(),
            max_file_size: 100 * 1024 * 1024, // 100MB
            temp_dir: "./downloads".to_string(),
            cleanup_interval: 3600, // 1 hour in seconds
            rate_limit_requests: 10,
            rate_limit_window: 60, // 1 minute
            cors_origins: vec!["http://localhost:3000".to_string()],
            recaptcha_secret_key: None,
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(port) = env::var("PORT") {
            config.port = port.parse().unwrap_or(config.port);
        }

        if let Ok(host) = env::var("HOST") {
            config.host = host;
        }

        if let Ok(max_size) = env::var("MAX_FILE_SIZE") {
            config.max_file_size = max_size.parse().unwrap_or(config.max_file_size);
        }

        if let Ok(temp_dir) = env::var("TEMP_DIR") {
            config.temp_dir = temp_dir;
        }

        if let Ok(cleanup_interval) = env::var("CLEANUP_INTERVAL") {
            config.cleanup_interval = cleanup_interval.parse().unwrap_or(config.cleanup_interval);
        }

        if let Ok(rate_limit) = env::var("RATE_LIMIT_REQUESTS") {
            config.rate_limit_requests = rate_limit.parse().unwrap_or(config.rate_limit_requests);
        }

        if let Ok(rate_window) = env::var("RATE_LIMIT_WINDOW") {
            config.rate_limit_window = rate_window.parse().unwrap_or(config.rate_limit_window);
        }

        if let Ok(origins) = env::var("CORS_ORIGINS") {
            config.cors_origins = origins.split(',').map(|s| s.trim().to_string()).collect();
        }

        // reCAPTCHA configuration
        if let Ok(secret_key) = env::var("RECAPTCHA_SECRET_KEY") {
            if !secret_key.is_empty() && secret_key != "your_recaptcha_secret_key_here" {
                config.recaptcha_secret_key = Some(secret_key);
            }
        }

        config
    }

    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn is_recaptcha_enabled(&self) -> bool {
        self.recaptcha_secret_key.is_some()
    }
}
