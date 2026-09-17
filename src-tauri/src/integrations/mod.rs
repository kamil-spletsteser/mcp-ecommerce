//! Kontrakt providerów i centralny rejestr. Nowa integracja = nowy moduł + jedna linia w `Registry::default()`;
//! rdzeń MCP i GUI nie wymagają zmian.

pub mod allegro;
pub mod baselinker;
pub mod http;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::config::Source;
use crate::secrets::{Secret, SecretKey, SecretStore};

/// Znormalizowane kody błędów zwracane agentowi AI i do GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    SourceNotFound,
    SourceDisabled,
    CredentialUnavailable,
    AuthFailed,
    RateLimited,
    UpstreamError,
    ValidationError,
    NotFound,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceNotFound => "SOURCE_NOT_FOUND",
            Self::SourceDisabled => "SOURCE_DISABLED",
            Self::CredentialUnavailable => "CREDENTIAL_UNAVAILABLE",
            Self::AuthFailed => "AUTH_FAILED",
            Self::RateLimited => "RATE_LIMITED",
            Self::UpstreamError => "UPSTREAM_ERROR",
            Self::ValidationError => "VALIDATION_ERROR",
            Self::NotFound => "NOT_FOUND",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ToolError {
    /// Komunikat zawsze przechodzi przez redakcję — błąd nie może wynieść sekretu.
    pub fn new(code: ErrorCode, message: impl AsRef<str>) -> Self {
        Self { code, message: crate::diagnostics::redact(message.as_ref()) }
    }
    pub fn validation(message: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::ValidationError, message)
    }
}

/// Jak źródło zdobywa dostęp do konta.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Wystarczą pola formularza (np. token API BaseLinkera).
    Fields,
    /// Pola formularza (dane aplikacji OAuth) + autoryzacja użytkownika w przeglądarce (OAuth 2.0 Device Flow).
    OauthDevice,
}

/// Pole formularza konfiguracji. `secret = true` → wartość trafia do credential store, nigdy do pliku;
/// `secret = false` → do `Source.settings`.
#[derive(Debug, Clone, Serialize)]
pub struct FieldSpec {
    pub key: &'static str,
    pub secret: bool,
    pub required: bool,
    pub max_len: usize,
    /// Niepuste = pole wyboru z zamkniętej listy (pierwsza pozycja jest domyślna); puste = dowolny tekst.
    pub options: &'static [&'static str],
    /// Czy zmiana wartości unieważnia połączenie (test od nowa, a dla OAuth skasowanie tokenów i ponowna autoryzacja).
    pub resets_auth: bool,
}

impl FieldSpec {
    pub const fn new(key: &'static str, secret: bool, required: bool, max_len: usize) -> Self {
        Self { key, secret, required, max_len, options: &[], resets_auth: true }
    }
    pub const fn options(mut self, options: &'static [&'static str]) -> Self {
        self.options = options;
        self
    }
    pub const fn keeps_auth(mut self) -> Self {
        self.resets_auth = false;
        self
    }
}

/// Możliwość opisana językiem użytkownika (tekst w i18n GUI pod kluczem `label_key`) + narzędzia MCP, które ją realizują.
#[derive(Debug, Clone, Serialize)]
pub struct Capability {
    pub label_key: &'static str,
    pub write: bool,
    pub tools: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub auth: AuthKind,
    pub fields: Vec<FieldSpec>,
    pub capabilities: Vec<Capability>,
}

pub struct ToolDef {
    /// Ostatni segment nazwy; pełna nazwa to `<provider>__<source_id>__<name>`.
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub read_only: bool,
}

/// Początek autoryzacji urządzeniowej. `device_code` zostaje w Ruście — GUI dostaje tylko kod użytkownika i adres.
pub struct DeviceAuthorization {
    pub device_code: Secret,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, PartialEq)]
pub enum AuthPoll {
    Pending,
    /// Serwer prosi o rzadsze odpytywanie.
    SlowDown,
    /// Użytkownik potwierdził; provider zapisał już tokeny w credential store.
    Done,
}

/// Kontekst wywołania: źródło + leniwy dostęp do jego sekretów.
pub struct SourceContext<'a> {
    pub source: &'a Source,
    pub secrets: &'a dyn SecretStore,
}

impl SourceContext<'_> {
    /// Sekret pobierany dopiero w momencie, gdy narzędzie go potrzebuje.
    pub fn secret(&self, kind: &str) -> Result<Secret, ToolError> {
        let key = SecretKey::new(&self.source.provider, &self.source.source_id, kind);
        match self.secrets.get(&key) {
            Ok(Some(secret)) => Ok(secret),
            Ok(None) => Err(ToolError::new(
                ErrorCode::CredentialUnavailable,
                format!("No credential stored for source '{}'. Re-enter it in the E-commerce MCP app.", self.source.source_id),
            )),
            Err(e) => Err(ToolError::new(ErrorCode::CredentialUnavailable, e.to_string())),
        }
    }

    pub fn set_secret(&self, kind: &str, value: &Secret) -> Result<(), ToolError> {
        let key = SecretKey::new(&self.source.provider, &self.source.source_id, kind);
        self.secrets.set(&key, value).map_err(|e| ToolError::new(ErrorCode::CredentialUnavailable, e.to_string()))
    }

    /// Niesekretne ustawienie źródła (pole formularza z `secret = false`).
    pub fn setting(&self, key: &str) -> Result<&str, ToolError> {
        self.source.settings[key].as_str().filter(|v| !v.is_empty()).ok_or_else(|| ToolError::validation(format!("Source setting '{key}' is missing.")))
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn meta(&self) -> ProviderMeta;
    /// Walidacja lokalna wartości pola formularza przed zapisem (format, długość) — bez sieci.
    fn validate_field(&self, _key: &str, _value: &str) -> Result<(), ToolError> {
        Ok(())
    }
    /// Sekrety zapisywane poza formularzem (np. tokeny OAuth) — kasowane razem ze źródłem i przy zmianie danych aplikacji.
    fn token_kinds(&self) -> &'static [&'static str] {
        &[]
    }
    /// `AuthKind::OauthDevice`: rozpoczęcie autoryzacji w przeglądarce.
    async fn begin_authorization(&self, _ctx: &SourceContext<'_>) -> Result<DeviceAuthorization, ToolError> {
        Err(ToolError::validation("This provider does not use browser authorization."))
    }
    /// `AuthKind::OauthDevice`: jedno odpytanie o wynik; przy `Done` provider zapisuje tokeny przez `ctx`.
    async fn poll_authorization(&self, _ctx: &SourceContext<'_>, _device_code: &Secret) -> Result<AuthPoll, ToolError> {
        Err(ToolError::validation("This provider does not use browser authorization."))
    }
    /// Tani, niezmieniający danych test. Zwraca krótki, bezpieczny opis sukcesu.
    async fn test_connection(&self, ctx: &SourceContext<'_>) -> Result<String, ToolError>;
    fn tools(&self) -> Vec<ToolDef>;
    async fn call_tool(&self, ctx: &SourceContext<'_>, tool: &str, args: &Value) -> Result<Value, ToolError>;
}

#[derive(Clone)]
pub struct Registry {
    providers: Vec<Arc<dyn Provider>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(vec![Arc::new(baselinker::BaseLinker::default()), Arc::new(allegro::Allegro::default())])
    }
}

impl Registry {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers }
    }
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.meta().id == id)
    }
    pub fn metas(&self) -> Vec<ProviderMeta> {
        self.providers.iter().map(|p| p.meta()).collect()
    }
}

const SEP: &str = "__";

pub fn tool_name(provider: &str, source_id: &str, tool: &str) -> String {
    format!("{provider}{SEP}{source_id}{SEP}{tool}")
}

/// `baselinker__glowny_sklep__list_orders` → (`baselinker`, `glowny_sklep`, `list_orders`).
pub fn parse_tool_name(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.splitn(3, SEP);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(p), Some(s), Some(t)) if !p.is_empty() && !s.is_empty() && !t.is_empty() => Some((p, s, t)),
        _ => None,
    }
}

// --- Walidacja argumentów narzędzi: wąskie typy, jawne zakresy, zero zgadywania ---

pub fn args_object<'a>(args: &'a Value, allowed: &[&str]) -> Result<&'a serde_json::Map<String, Value>, ToolError> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    let map = match args {
        Value::Null => EMPTY.get_or_init(Default::default),
        Value::Object(map) => map,
        _ => return Err(ToolError::validation("Arguments must be a JSON object.")),
    };
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(unknown) => Err(ToolError::validation(format!("Unknown argument '{unknown}'. Allowed: {}.", allowed.join(", ")))),
        None => Ok(map),
    }
}

pub fn opt_int(map: &serde_json::Map<String, Value>, key: &str, min: i64, max: i64) -> Result<Option<i64>, ToolError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(ToolError::validation(format!("'{key}' must be an integer between {min} and {max}."))),
        },
    }
}

pub fn req_int(map: &serde_json::Map<String, Value>, key: &str, min: i64, max: i64) -> Result<i64, ToolError> {
    opt_int(map, key, min, max)?.ok_or_else(|| ToolError::validation(format!("'{key}' is required.")))
}

pub fn opt_str<'a>(map: &'a serde_json::Map<String, Value>, key: &str, max_len: usize) -> Result<Option<&'a str>, ToolError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= max_len => Ok(Some(s.trim())),
        _ => Err(ToolError::validation(format!("'{key}' must be a non-empty string of at most {max_len} characters."))),
    }
}

pub fn opt_enum<'a>(map: &'a serde_json::Map<String, Value>, key: &str, allowed: &[&str]) -> Result<Option<&'a str>, ToolError> {
    match opt_str(map, key, 64)? {
        None => Ok(None),
        Some(value) if allowed.contains(&value) => Ok(Some(value)),
        Some(_) => Err(ToolError::validation(format!("'{key}' must be one of: {}.", allowed.join(", ")))),
    }
}

/// Data jako `YYYY-MM-DD` (UTC; `end_of_day` → 23:59:59) albo RFC 3339. Zwraca unix timestamp; nie wcześniej niż 2000-01-01.
pub fn opt_date(map: &serde_json::Map<String, Value>, key: &str, end_of_day: bool) -> Result<Option<i64>, ToolError> {
    let Some(input) = opt_str(map, key, 40)? else { return Ok(None) };
    let format = time::macros::format_description!("[year]-[month]-[day]");
    let timestamp = if let Ok(date) = time::Date::parse(input, &format) {
        date.midnight().assume_utc().unix_timestamp() + if end_of_day { 86_399 } else { 0 }
    } else if let Ok(datetime) = time::OffsetDateTime::parse(input, &time::format_description::well_known::Rfc3339) {
        datetime.unix_timestamp()
    } else {
        return Err(ToolError::validation(format!("'{key}' must be YYYY-MM-DD or an RFC 3339 datetime, e.g. 2026-01-31 or 2026-01-31T12:00:00Z.")));
    };
    // 946684800 = 2000-01-01
    if timestamp < 946_684_800 {
        return Err(ToolError::validation(format!("'{key}' must not be earlier than 2000-01-01.")));
    }
    Ok(Some(timestamp))
}

/// Unix timestamp → RFC 3339 (UTC). `None` dla wartości ≤ 0 (u providerów „brak daty”).
pub fn iso(timestamp: i64) -> Option<String> {
    if timestamp <= 0 {
        return None;
    }
    time::OffsetDateTime::from_unix_timestamp(timestamp).ok()?.format(&time::format_description::well_known::Rfc3339).ok()
}

pub fn object_schema(properties: Value, required: &[&str]) -> Value {
    serde_json::json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

/// Whitelist pól: do modelu trafia tylko to, co wymienione; puste wartości pomijamy.
pub fn pick(raw: &Value, fields: &[&str]) -> serde_json::Map<String, Value> {
    fields
        .iter()
        .filter_map(|&field| {
            let value = raw.get(field)?;
            let empty = value.is_null() || value.as_str().is_some_and(|s| s.is_empty());
            (!empty).then(|| (field.to_string(), value.clone()))
        })
        .collect()
}

pub fn pick_each(list: &Value, fields: &[&str]) -> Vec<Value> {
    list.as_array().map(|items| items.iter().map(|item| Value::Object(pick(item, fields))).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_contains_baselinker_and_allegro() {
        let registry = Registry::default();
        let metas = registry.metas();
        assert_eq!(metas.iter().map(|m| (m.id, m.auth)).collect::<Vec<_>>(), vec![("baselinker", AuthKind::Fields), ("allegro", AuthKind::OauthDevice)]);
        assert!(registry.get("shopify").is_none());
    }

    #[test]
    fn provider_contract_holds_for_every_provider() {
        for provider in Registry::default().providers {
            let meta = provider.meta();
            let tools = provider.tools();
            assert!(!tools.is_empty(), "{}: provider bez narzędzi", meta.id);
            assert!(meta.fields.iter().any(|f| f.secret), "{}: brak pola sekretu", meta.id);
            // tokeny OAuth istnieją wtedy i tylko wtedy, gdy provider autoryzuje się w przeglądarce; nie kolidują z polami formularza
            assert_eq!(meta.auth == AuthKind::OauthDevice, !provider.token_kinds().is_empty(), "{}", meta.id);
            assert!(provider.token_kinds().iter().all(|kind| meta.fields.iter().all(|f| f.key != *kind)), "{}", meta.id);
            let names: Vec<_> = tools.iter().map(|t| t.name).collect();
            for tool in &tools {
                assert!(!tool.name.contains(SEP) && tool.description.len() > 20, "{}", tool.name);
                assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
                assert_eq!(tool.input_schema["additionalProperties"], false, "{}", tool.name);
                // limit nazw narzędzi u klientów MCP: 64 znaki, przy source_id do 20 znaków (+ sufiks _NN)
                assert!(tool_name(meta.id, &"x".repeat(23), tool.name).len() <= 64, "{}", tool.name);
            }
            for capability in &meta.capabilities {
                for tool in &capability.tools {
                    assert!(names.contains(tool), "{}: capability wskazuje nieistniejące narzędzie {tool}", meta.id);
                }
                let any_write = capability.tools.iter().any(|t| !tools.iter().find(|d| d.name == *t).unwrap().read_only);
                assert_eq!(capability.write, any_write, "{}: {}", meta.id, capability.label_key);
            }
        }
    }

    #[test]
    fn tool_names_roundtrip() {
        let name = tool_name("baselinker", "glowny_sklep", "list_orders");
        assert_eq!(name, "baselinker__glowny_sklep__list_orders");
        assert_eq!(parse_tool_name(&name), Some(("baselinker", "glowny_sklep", "list_orders")));
        assert_eq!(parse_tool_name("list_orders"), None);
        assert_eq!(parse_tool_name("a____b"), None);
    }

    #[test]
    fn argument_validation() {
        let args = json!({"limit": 5, "name": " abc "});
        let map = args_object(&args, &["limit", "name"]).unwrap();
        assert_eq!(opt_int(map, "limit", 1, 100).unwrap(), Some(5));
        assert_eq!(opt_str(map, "name", 10).unwrap(), Some("abc"));
        assert_eq!(opt_int(map, "missing", 1, 100).unwrap(), None);
        assert!(opt_int(map, "limit", 10, 100).is_err());
        assert!(req_int(map, "missing", 1, 2).is_err());
        assert_eq!(args_object(&args, &["limit"]).unwrap_err().code, ErrorCode::ValidationError);
        assert!(args_object(&json!([1]), &[]).is_err());
        assert!(args_object(&Value::Null, &[]).unwrap().is_empty());
        assert!(opt_int(args_object(&json!({"limit": "5"}), &["limit"]).unwrap(), "limit", 1, 100).is_err());

        let args = json!({"status": "SENT", "from": "2025-06-15", "to": "2025-06-15", "bad": "15.06.2025", "old": "1999-12-31"});
        let map = args.as_object().unwrap();
        assert_eq!(opt_enum(map, "status", &["NEW", "SENT"]).unwrap(), Some("SENT"));
        assert!(opt_enum(map, "status", &["NEW"]).is_err());
        assert_eq!(opt_date(map, "from", false).unwrap(), Some(1_749_945_600));
        assert_eq!(opt_date(map, "to", true).unwrap(), Some(1_749_945_600 + 86_399));
        assert!(opt_date(map, "bad", false).is_err() && opt_date(map, "old", false).is_err());
        assert_eq!(iso(1_750_000_000).as_deref(), Some("2025-06-15T15:06:40Z"));
        assert_eq!(iso(0), None);
    }

    #[test]
    fn tool_error_messages_are_redacted() {
        crate::diagnostics::register_secret("token-w-bledzie-9876");
        let err = ToolError::new(ErrorCode::UpstreamError, "upstream said: token-w-bledzie-9876");
        assert!(!err.message.contains("9876"));
    }
}
