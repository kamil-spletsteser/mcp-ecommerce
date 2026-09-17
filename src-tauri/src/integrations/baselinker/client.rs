//! Klient HTTP BaseLinker API (https://api.baselinker.com/, stan dokumentacji: 2026-09).
//! Jeden stały endpoint — adresu nie da się podać z zewnątrz (brak SSRF / dowolnego proxy).

use std::time::Duration;

use serde_json::Value;

use crate::integrations::http::{self, Attempt};
use crate::integrations::{ErrorCode, ToolError};
use crate::secrets::Secret;

pub const API_URL: &str = "https://api.baselinker.com/connector.php";
const SERVICE: &str = "BaseLinker";

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    backoff: Duration,
}

impl Client {
    pub fn new(base_url: String, timeout: Duration, backoff: Duration) -> Self {
        Self { http: http::client(timeout), base_url, backoff }
    }

    /// Odczyty (`idempotent`) są ponawiane wg `http::with_retries`; zapisy nigdy. Limit API (100 req/min) wraca jako `RATE_LIMITED`.
    pub async fn call(&self, token: &Secret, method: &'static str, params: Value, idempotent: bool) -> Result<Value, ToolError> {
        http::with_retries(&format!("baselinker {method}"), idempotent, self.backoff, || self.call_once(token, method, &params)).await
    }

    async fn call_once(&self, token: &Secret, method: &str, params: &Value) -> Attempt<Value> {
        let response = self
            .http
            .post(&self.base_url)
            .header("X-BLToken", token.expose())
            .form(&[("method", method), ("parameters", &params.to_string())])
            .send()
            .await
            .map_err(|e| http::send_error(&e, SERVICE))?;

        let status = response.status();
        match status.as_u16() {
            401 | 403 => return Err((ToolError::new(ErrorCode::AuthFailed, format!("BaseLinker rejected the API token (HTTP {status}).")), false)),
            429 => return Err((rate_limited(), false)),
            500..=599 => return Err(http::upstream(format!("BaseLinker is unavailable (HTTP {status})."), true)),
            200..=299 => {}
            _ => return Err(http::upstream(format!("Unexpected BaseLinker response (HTTP {status})."), false)),
        }

        let body = http::read_capped(response, SERVICE).await?;
        let json: Value = serde_json::from_slice(&body).map_err(|_| http::upstream("BaseLinker returned a non-JSON response.", false))?;
        if json["status"] == "SUCCESS" {
            return Ok(json);
        }
        let code = json["error_code"].as_str().unwrap_or("ERROR_UNKNOWN");
        let message = json["error_message"].as_str().unwrap_or("");
        Err((classify(code, message), false))
    }
}

fn rate_limited() -> ToolError {
    ToolError::new(ErrorCode::RateLimited, "BaseLinker API limit reached (100 requests/minute). Wait a minute and try again.")
}

/// Mapowanie `error_code` BaseLinkera na znormalizowane kody. Dokumentacja nie publikuje pełnej listy kodów,
/// więc klasyfikujemy po fragmentach nazw (np. `ERROR_BAD_TOKEN`, `ERROR_USER_ACCOUNT_BLOCKED`, `ERROR_..._LIMIT`).
pub fn classify(code: &str, message: &str) -> ToolError {
    let upper = code.to_uppercase();
    if ["TOKEN", "AUTH", "BLOCKED"].iter().any(|k| upper.contains(k)) {
        ToolError::new(ErrorCode::AuthFailed, format!("BaseLinker rejected the API token ({code})."))
    } else if ["LIMIT", "TOO_MANY"].iter().any(|k| upper.contains(k)) {
        rate_limited()
    } else {
        ToolError::new(ErrorCode::UpstreamError, format!("BaseLinker error {code}: {message}"))
    }
}
