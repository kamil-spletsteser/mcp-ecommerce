//! Wspólne zasady HTTP dla providerów: timeouty, limit rozmiaru odpowiedzi, ponowienia tylko dla odczytów.

use std::future::Future;
use std::time::Duration;

use super::{ErrorCode, ToolError};

/// Górny limit rozmiaru odpowiedzi upstream.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETRIES: u32 = 2;

/// Wynik jednej próby: Err = (błąd, czy przejściowy — tylko takie wolno ponawiać).
pub type Attempt<T> = Result<T, (ToolError, bool)>;

pub fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("ecommerce-mcp/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static reqwest client configuration")
}

pub fn upstream(message: impl AsRef<str>, transient: bool) -> (ToolError, bool) {
    (ToolError::new(ErrorCode::UpstreamError, message), transient)
}

/// Błąd wysyłki. Bez `e.to_string()` — treść błędu reqwest mogłaby zawierać szczegóły żądania.
pub fn send_error(e: &reqwest::Error, service: &str) -> (ToolError, bool) {
    let what = if e.is_timeout() { "timed out" } else { "failed (network error)" };
    upstream(format!("Request to {service} {what}."), true)
}

pub async fn read_capped(mut response: reqwest::Response, service: &str) -> Attempt<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| upstream(format!("Reading {service} response failed."), true))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(upstream(format!("{service} response too large; narrow the filters."), false));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `idempotent = true` (odczyty): do 2 ponowień z rosnącym backoffem dla błędów przejściowych (timeout, sieć, 5xx).
/// Zapisy nigdy nie są ponawiane automatycznie.
pub async fn with_retries<T, F, Fut>(label: &str, idempotent: bool, backoff: Duration, mut attempt: F) -> Result<T, ToolError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let mut retries = 0;
    loop {
        match attempt().await {
            Err((error, true)) if idempotent && retries < MAX_RETRIES => {
                retries += 1;
                crate::diagnostics::log(&format!("{label}: {} — retry {retries}/{MAX_RETRIES}", error.message));
                tokio::time::sleep(backoff * 3u32.pow(retries - 1)).await;
            }
            Err((error, _)) => return Err(error),
            Ok(value) => return Ok(value),
        }
    }
}
