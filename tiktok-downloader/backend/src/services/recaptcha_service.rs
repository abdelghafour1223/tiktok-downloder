use anyhow::{anyhow, Result};
use reqwest::Client;
use std::collections::HashMap;
use tracing::{info, warn, error};
use crate::models::{RecaptchaVerifyRequest, RecaptchaVerifyResponse};

#[derive(Clone)]
pub struct RecaptchaService {
    client: Client,
    secret_key: Option<String>,
}

impl RecaptchaService {
    pub fn new(secret_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            secret_key,
        }
    }

    /// Verify a reCAPTCHA token with Google's API
    pub async fn verify_token(&self, token: &str, remote_ip: Option<String>) -> Result<bool> {
        // If no secret key is configured, skip verification (for development)
        let secret_key = match &self.secret_key {
            Some(key) => key,
            None => {
                warn!("reCAPTCHA secret key not configured, skipping verification");
                return Ok(true);
            }
        };

        if token.is_empty() {
            return Err(anyhow!("reCAPTCHA token is empty"));
        }

        info!("Verifying reCAPTCHA token with Google API");

        // Prepare form data for Google's siteverify API
        let mut form_data = HashMap::new();
        form_data.insert("secret", secret_key.as_str());
        form_data.insert("response", token);
        
        if let Some(ref ip) = remote_ip {
            form_data.insert("remoteip", ip.as_str());
        }

        // Make request to Google's siteverify API
        let response = self
            .client
            .post("https://www.google.com/recaptcha/api/siteverify")
            .form(&form_data)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send reCAPTCHA verification request: {}", e))?;

        if !response.status().is_success() {
            error!("reCAPTCHA API returned error status: {}", response.status());
            return Err(anyhow!("reCAPTCHA verification API error: {}", response.status()));
        }

        let verify_response: RecaptchaVerifyResponse = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse reCAPTCHA response: {}", e))?;

        if verify_response.success {
            info!("reCAPTCHA verification successful");
            Ok(true)
        } else {
            warn!(
                "reCAPTCHA verification failed. Error codes: {:?}",
                verify_response.error_codes
            );
            
            // Log specific error codes for debugging
            if let Some(error_codes) = &verify_response.error_codes {
                for code in error_codes {
                    match code.as_str() {
                        "missing-input-secret" => error!("reCAPTCHA secret key is missing"),
                        "invalid-input-secret" => error!("reCAPTCHA secret key is invalid"),
                        "missing-input-response" => error!("reCAPTCHA token is missing"),
                        "invalid-input-response" => error!("reCAPTCHA token is invalid or expired"),
                        "bad-request" => error!("Bad request to reCAPTCHA API"),
                        "timeout-or-duplicate" => error!("reCAPTCHA token has timed out or been used already"),
                        _ => error!("Unknown reCAPTCHA error code: {}", code),
                    }
                }
            }
            
            Err(anyhow!("reCAPTCHA verification failed"))
        }
    }

    /// Check if reCAPTCHA verification is required (secret key is configured)
    pub fn is_enabled(&self) -> bool {
        self.secret_key.is_some()
    }

    /// Get human-readable error message for reCAPTCHA verification failure
    pub fn get_error_message(error_codes: Option<&Vec<String>>) -> String {
        match error_codes {
            Some(codes) if !codes.is_empty() => {
                match codes[0].as_str() {
                    "missing-input-response" => "Please complete the reCAPTCHA challenge".to_string(),
                    "invalid-input-response" => "reCAPTCHA verification failed. Please try again".to_string(),
                    "timeout-or-duplicate" => "reCAPTCHA has expired. Please refresh and try again".to_string(),
                    _ => "reCAPTCHA verification failed. Please try again".to_string(),
                }
            }
            _ => "reCAPTCHA verification failed. Please try again".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recaptcha_service_creation() {
        let service = RecaptchaService::new(Some("test_secret".to_string()));
        assert!(service.is_enabled());

        let service_no_key = RecaptchaService::new(None);
        assert!(!service_no_key.is_enabled());
    }

    #[test]
    fn test_error_message_generation() {
        let error_codes = vec!["invalid-input-response".to_string()];
        let message = RecaptchaService::get_error_message(Some(&error_codes));
        assert!(message.contains("reCAPTCHA verification failed"));

        let message_empty = RecaptchaService::get_error_message(None);
        assert!(message_empty.contains("reCAPTCHA verification failed"));
    }
}
