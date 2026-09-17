//! OAuth 2.0 Device Flow + odświeżanie tokenów Allegro (https://developer.allegro.pl/tutorials/uwierzytelnianie-i-autoryzacja-zlq9e75GdIR).
//!
//! Sekrety źródła (osobne wpisy credential store — wpis Windows mieści 2560 bajtów, a tokeny to JWT ~1–1,5 KB):
//! `client_secret` (z formularza), `refresh_token` (3 mies., JEDNORAZOWY — każda odpowiedź niesie nowy), `access_token` (12 h).

use serde_json::Value;

use super::Allegro;
use crate::integrations::http::{self, Attempt};
use crate::integrations::{AuthPoll, DeviceAuthorization, ErrorCode, SourceContext, ToolError};
use crate::secrets::Secret;

pub const CLIENT_ID: &str = "client_id";
pub const CLIENT_SECRET: &str = "client_secret";
pub const REFRESH_TOKEN: &str = "refresh_token";
pub const ACCESS_TOKEN: &str = "access_token";
/// Tylko odczyt: zamówienia, oferty, profil.
const SCOPES: &str = "allegro:api:orders:read allegro:api:sale:offers:read allegro:api:profile:read";
const SERVICE: &str = "Allegro";

fn reconnect() -> ToolError {
    ToolError::new(ErrorCode::AuthFailed, "Allegro authorization expired or was revoked. Reconnect the account in the E-commerce MCP app.")
}

impl Allegro {
    /// POST na endpoint OAuth z Basic auth aplikacji. Ok = (status HTTP, JSON).
    async fn oauth_post(&self, ctx: &SourceContext<'_>, path: &str, form: &[(&str, &str)]) -> Attempt<(u16, Value)> {
        let client_id = ctx.setting(CLIENT_ID).map_err(|e| (e, false))?;
        let client_secret = ctx.secret(CLIENT_SECRET).map_err(|e| (e, false))?;
        let response = self
            .http
            .post(format!("{}{path}", self.auth_base))
            .basic_auth(client_id, Some(client_secret.expose()))
            .header("Accept", "application/json")
            .form(form)
            .send()
            .await
            .map_err(|e| http::send_error(&e, SERVICE))?;
        let status = response.status().as_u16();
        if status >= 500 {
            return Err(http::upstream(format!("Allegro sign-in service is unavailable (HTTP {status})."), true));
        }
        let body = http::read_capped(response, SERVICE).await?;
        let json = serde_json::from_slice(&body).map_err(|_| http::upstream("Allegro sign-in service returned a non-JSON response.", false))?;
        Ok((status, json))
    }

    pub(super) async fn begin(&self, ctx: &SourceContext<'_>) -> Result<DeviceAuthorization, ToolError> {
        let client_id = ctx.setting(CLIENT_ID)?;
        let (status, json) = self.oauth_post(ctx, "/auth/oauth/device", &[("client_id", client_id), ("scope", SCOPES)]).await.map_err(|(e, _)| e)?;
        if status != 200 {
            return Err(ToolError::new(
                ErrorCode::AuthFailed,
                "Allegro rejected the Client ID / Client Secret. Check them and make sure the app is registered as a device-type application.",
            ));
        }
        let text = |key: &str| json[key].as_str().filter(|v| !v.is_empty());
        let (Some(device_code), Some(user_code), Some(uri)) =
            (text("device_code"), text("user_code"), text("verification_uri_complete").or(text("verification_uri")))
        else {
            return Err(ToolError::new(ErrorCode::UpstreamError, "Allegro returned an incomplete device authorization response."));
        };
        // Adres otworzymy w przeglądarce użytkownika — tylko jeśli prowadzi do serwisu logowania Allegro.
        if !uri.starts_with(&format!("{}/", self.auth_base)) {
            return Err(ToolError::new(ErrorCode::UpstreamError, "Allegro returned an unexpected verification address."));
        }
        Ok(DeviceAuthorization {
            device_code: Secret::new(device_code.to_string()),
            user_code: user_code.to_string(),
            verification_uri: uri.to_string(),
            interval_secs: json["interval"].as_u64().unwrap_or(5).clamp(1, 60),
            expires_in_secs: json["expires_in"].as_u64().unwrap_or(600).clamp(60, 3600),
        })
    }

    pub(super) async fn poll(&self, ctx: &SourceContext<'_>, device_code: &Secret) -> Result<AuthPoll, ToolError> {
        let form = [("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", device_code.expose())];
        let (status, json) = self.oauth_post(ctx, "/auth/oauth/token", &form).await.map_err(|(e, _)| e)?;
        if status == 200 {
            store_tokens(ctx, &json)?;
            return Ok(AuthPoll::Done);
        }
        if status == 429 {
            return Ok(AuthPoll::SlowDown);
        }
        match json["error"].as_str().unwrap_or("") {
            "authorization_pending" => Ok(AuthPoll::Pending),
            "slow_down" => Ok(AuthPoll::SlowDown),
            "access_denied" => Err(ToolError::new(ErrorCode::AuthFailed, "Access was denied in Allegro.")),
            _ => Err(ToolError::new(ErrorCode::AuthFailed, "The authorization code expired or is no longer valid. Start the connection again.")),
        }
    }

    /// Blokada odświeżania wspólna dla wszystkich procesów aplikacji (GUI + sesje MCP) i wywołań w tym procesie:
    /// refresh token jest jednorazowy, więc dwa równoległe odświeżenia unieważniłyby sobie nawzajem tokeny.
    /// Zwolnienie = zamknięcie pliku (także gdy proces padnie). Gdy pliku nie da się założyć, działamy bez blokady.
    async fn refresh_lock(&self) -> Option<std::fs::File> {
        let path = self.lock_path.clone();
        let locked = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::options().create(true).write(true).truncate(false).open(path)?;
            file.lock()?;
            Ok::<_, std::io::Error>(file)
        });
        match locked.await {
            Ok(Ok(file)) => Some(file),
            _ => {
                crate::diagnostics::log("allegro: token refresh lock unavailable, continuing without it");
                None
            }
        }
    }

    /// Wymienia refresh token na nową parę. `used_access` = token, który właśnie dostał 401.
    pub(super) async fn refresh(&self, ctx: &SourceContext<'_>, used_access: &Secret) -> Result<Secret, ToolError> {
        let _lock = self.refresh_lock().await;
        // Ktoś już odświeżył, zanim dostaliśmy blokadę?
        let stored_access = ctx.secret(ACCESS_TOKEN)?;
        if stored_access.expose() != used_access.expose() {
            return Ok(stored_access);
        }
        let refresh_token = ctx.secret(REFRESH_TOKEN)?;
        let form = [("grant_type", "refresh_token"), ("refresh_token", refresh_token.expose())];
        let (status, json) = http::with_retries("allegro token refresh", true, self.backoff, || self.oauth_post(ctx, "/auth/oauth/token", &form)).await?;
        if status == 200 {
            return store_tokens(ctx, &json);
        }
        // Siatka bezpieczeństwa na wypadek pracy bez blokady: odrzucony refresh token mógł zostać chwilę wcześniej zużyty
        // przez inny proces — wtedy w credential store leży już nowa para.
        if ctx.secret(REFRESH_TOKEN)?.expose() != refresh_token.expose() {
            return ctx.secret(ACCESS_TOKEN);
        }
        Err(reconnect())
    }
}

/// Najpierw refresh token: gdyby proces padł między zapisami, stary access token da 401 → odświeżenie nowym refresh tokenem zadziała.
/// Odwrotna kolejność mogłaby zostawić w store zużyty (martwy) refresh token.
fn store_tokens(ctx: &SourceContext<'_>, json: &Value) -> Result<Secret, ToolError> {
    let (Some(access), Some(refresh)) = (json["access_token"].as_str(), json["refresh_token"].as_str()) else {
        return Err(ToolError::new(ErrorCode::UpstreamError, "Allegro returned an incomplete token response."));
    };
    let access = Secret::new(access.to_string());
    ctx.set_secret(REFRESH_TOKEN, &Secret::new(refresh.to_string()))?;
    ctx.set_secret(ACCESS_TOKEN, &access)?;
    Ok(access)
}

pub(super) fn not_connected(error: ToolError) -> ToolError {
    match error.code {
        ErrorCode::CredentialUnavailable => {
            ToolError::new(ErrorCode::CredentialUnavailable, "This Allegro account is not connected yet. Finish the connection in the E-commerce MCP app.")
        }
        _ => error,
    }
}

pub(super) fn unauthorized_after_refresh() -> ToolError {
    reconnect()
}
