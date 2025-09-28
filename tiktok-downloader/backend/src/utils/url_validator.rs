use regex::Regex;
use url::Url;

pub fn is_valid_tiktok_url(url: &str) -> bool {
    // First check if it's a valid URL
    if Url::parse(url).is_err() {
        return false;
    }

    // TikTok URL patterns to match
    let patterns = vec![
        r"^https?://(www\.)?tiktok\.com/@[^/]+/video/\d+",
        r"^https?://vm\.tiktok\.com/[A-Za-z0-9]+/?",
        r"^https?://(www\.)?tiktok\.com/t/[A-Za-z0-9]+/?",
        r"^https?://m\.tiktok\.com/v/\d+\.html",
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            if re.is_match(url) {
                return true;
            }
        }
    }

    false
}

/// Validates TikTok profile URLs (e.g., https://www.tiktok.com/@username)
pub fn is_valid_tiktok_profile_url(url: &str) -> bool {
    // First check if it's a valid URL
    if Url::parse(url).is_err() {
        return false;
    }

    // TikTok profile URL patterns
    let patterns = vec![
        r"^https?://(www\.)?tiktok\.com/@[A-Za-z0-9_.]+/?",
        r"^https?://(www\.)?tiktok\.com/@[A-Za-z0-9_.]+$",
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            if re.is_match(url) {
                return true;
            }
        }
    }

    false
}

/// Extracts username from TikTok profile URL
pub fn extract_tiktok_username(profile_url: &str) -> Option<String> {
    if !is_valid_tiktok_profile_url(profile_url) {
        return None;
    }

    if let Ok(re) = Regex::new(r"@([A-Za-z0-9_.]+)") {
        if let Some(captures) = re.captures(profile_url) {
            if let Some(username) = captures.get(1) {
                return Some(username.as_str().to_string());
            }
        }
    }

    None
}

pub fn normalize_tiktok_url(url: &str) -> Option<String> {
    if !is_valid_tiktok_url(url) {
        return None;
    }

    // Convert short URLs to standard format if needed
    // This is a simplified version - in practice you might need to follow redirects
    if url.contains("vm.tiktok.com") || url.contains("tiktok.com/t/") {
        // For short URLs, you would typically need to follow the redirect
        // to get the canonical URL. For now, return as-is.
        return Some(url.to_string());
    }

    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_tiktok_urls() {
        let valid_urls = vec![
            "https://www.tiktok.com/@username/video/1234567890123456789",
            "https://tiktok.com/@username/video/1234567890123456789",
            "https://vm.tiktok.com/ZTdXXXXXX/",
            "https://www.tiktok.com/t/ZTdXXXXXX/",
        ];

        for url in valid_urls {
            assert!(is_valid_tiktok_url(url), "URL should be valid: {}", url);
        }
    }

    #[test]
    fn test_invalid_urls() {
        let invalid_urls = vec![
            "https://youtube.com/watch?v=123",
            "https://instagram.com/p/123",
            "not-a-url",
            "https://tiktok.com/invalid",
        ];

        for url in invalid_urls {
            assert!(!is_valid_tiktok_url(url), "URL should be invalid: {}", url);
        }
    }

    #[test]
    fn test_valid_tiktok_profile_urls() {
        let valid_profile_urls = vec![
            "https://www.tiktok.com/@username",
            "https://tiktok.com/@username",
            "https://www.tiktok.com/@user_name",
            "https://www.tiktok.com/@user.name",
            "https://www.tiktok.com/@user123",
            "https://www.tiktok.com/@username/",
        ];

        for url in valid_profile_urls {
            assert!(is_valid_tiktok_profile_url(url), "Profile URL should be valid: {}", url);
        }
    }

    #[test]
    fn test_invalid_profile_urls() {
        let invalid_profile_urls = vec![
            "https://www.tiktok.com/@",
            "https://www.tiktok.com/username",
            "https://youtube.com/@username",
            "not-a-url",
            "https://www.tiktok.com/@username/video/123",
        ];

        for url in invalid_profile_urls {
            assert!(!is_valid_tiktok_profile_url(url), "Profile URL should be invalid: {}", url);
        }
    }

    #[test]
    fn test_extract_username() {
        let test_cases = vec![
            ("https://www.tiktok.com/@testuser", Some("testuser".to_string())),
            ("https://tiktok.com/@user_name", Some("user_name".to_string())),
            ("https://www.tiktok.com/@user.123", Some("user.123".to_string())),
            ("https://www.tiktok.com/@username/", Some("username".to_string())),
            ("invalid-url", None),
            ("https://youtube.com/@user", None),
        ];

        for (url, expected) in test_cases {
            assert_eq!(extract_tiktok_username(url), expected, "Username extraction failed for: {}", url);
        }
    }
}
