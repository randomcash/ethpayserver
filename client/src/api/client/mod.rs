//! HTTP client for ethpayserver API.

use gloo_net::http::{Request, RequestBuilder};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

mod admin;
mod invoices;
mod payments;
mod stores;

/// API client errors.
#[derive(Error, Debug, Clone)]
pub enum ApiError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("HTTP error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Unauthorized")]
    Unauthorized,
}

/// API client for ethpayserver.
#[derive(Clone)]
pub struct EvmApiClient {
    base_url: String,
    token: Option<String>,
}

impl EvmApiClient {
    /// Create a new API client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
        }
    }

    /// Create a client for public endpoints (no auth header sent).
    ///
    /// Uses a same-origin relative base URL. Call `with_token` to authenticate later.
    pub fn unauthenticated() -> Self {
        Self::new("")
    }

    /// Set the authorization token.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// Build a request with authentication.
    fn build_request(&self, method: &str, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let builder = match method {
            "GET" => Request::get(&url),
            "POST" => Request::post(&url),
            "PUT" => Request::put(&url),
            "DELETE" => Request::delete(&url),
            "PATCH" => Request::patch(&url),
            _ => Request::get(&url),
        };

        let builder = if let Some(ref token) = self.token {
            builder.header("Authorization", &format!("Bearer {}", token))
        } else {
            builder
        };

        builder.header("Content-Type", "application/json")
    }

    /// Make a GET request.
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let request = self
            .build_request("GET", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a GET request returning raw text (for CSV downloads).
    async fn get_text(&self, path: &str) -> Result<String, ApiError> {
        let request = self
            .build_request("GET", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }

        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }

        response
            .text()
            .await
            .map_err(|e| ApiError::Parse(e.to_string()))
    }

    /// Make a POST request.
    async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self
            .build_request("POST", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a PUT request.
    async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self
            .build_request("PUT", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a PATCH request.
    async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self
            .build_request("PATCH", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a DELETE request.
    async fn delete(&self, path: &str) -> Result<(), ApiError> {
        let request = self
            .build_request("DELETE", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }

        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }

        Ok(())
    }

    /// Make a POST request without a body, returning parsed JSON.
    async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let request = self
            .build_request("POST", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        self.handle_response(response).await
    }

    /// Make a POST request without a body, ignoring response body.
    async fn post_empty_body(&self, path: &str) -> Result<(), ApiError> {
        let request = self
            .build_request("POST", path)
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }
        Ok(())
    }

    /// Make a PATCH request with body, ignoring response body.
    async fn patch_empty<B: Serialize>(&self, path: &str, body: &B) -> Result<(), ApiError> {
        let request = self
            .build_request("PATCH", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }
        Ok(())
    }

    /// Make a PUT request with body, ignoring response body.
    async fn put_empty<B: Serialize>(&self, path: &str, body: &B) -> Result<(), ApiError> {
        let request = self
            .build_request("PUT", path)
            .json(body)
            .map_err(|e| ApiError::Parse(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }
        Ok(())
    }

    /// Handle response and parse JSON.
    async fn handle_response<T: DeserializeOwned>(
        &self,
        response: gloo_net::http::Response,
    ) -> Result<T, ApiError> {
        if response.status() == 401 {
            return Err(ApiError::Unauthorized);
        }

        if !response.ok() {
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: response.status(),
                message,
            });
        }

        response
            .json()
            .await
            .map_err(|e| ApiError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_client_new() {
        let client = EvmApiClient::new("http://localhost:5000");
        assert_eq!(client.base_url, "http://localhost:5000");
        assert_eq!(client.token, None);
    }

    #[test]
    fn test_api_client_with_token() {
        let client =
            EvmApiClient::new("http://localhost:5000").with_token(Some("test-token".to_string()));

        assert_eq!(client.token, Some("test-token".to_string()));
    }

    #[test]
    fn test_api_error_display() {
        let err = ApiError::Http {
            status: 404,
            message: "Not Found".to_string(),
        };
        assert_eq!(err.to_string(), "HTTP error 404: Not Found");

        let err = ApiError::Unauthorized;
        assert_eq!(err.to_string(), "Unauthorized");
    }
}
