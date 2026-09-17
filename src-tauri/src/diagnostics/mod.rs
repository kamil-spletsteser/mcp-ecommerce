//! Redakcja sekretów, logowanie na stderr i raport diagnostyczny.

use std::sync::Mutex;

use crate::config::Config;

const REDACTED: &str = "[REDACTED]";

/// Sekrety załadowane w tym procesie — wycinane z każdego logu i komunikatu błędu.
static KNOWN_SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn register_secret(value: &str) {
    if value.len() < 4 {
        return;
    }
    let mut known = KNOWN_SECRETS.lock().unwrap_or_else(|e| e.into_inner());
    if !known.iter().any(|s| s == value) {
        known.push(value.to_string());
    }
}

/// Usuwa z tekstu znane sekrety oraz wszystko, co wygląda jak token
/// (ciąg ≥32 znaków alfanumerycznych zawierający litery i cyfry).
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for secret in KNOWN_SECRETS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        out = out.replace(secret.as_str(), REDACTED);
    }
    redact_token_like(&out)
}

fn redact_token_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let token_like = run.len() >= 32 && run.chars().any(|c| c.is_ascii_digit()) && run.chars().any(|c| c.is_ascii_alphabetic());
        out.push_str(if token_like { REDACTED } else { run });
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Jedyny kanał logów. W trybie MCP stdout należy do protokołu, więc logujemy wyłącznie na stderr.
pub fn log(message: &str) {
    eprintln!("[ecommerce-mcp] {}", redact(message));
}

pub struct ReportInput<'a> {
    pub config: &'a Config,
    pub data_dir: &'a std::path::Path,
    pub credential_store: &'a Result<(), String>,
    pub mcp_status: &'a str,
}

/// Raport do skopiowania przez użytkownika: bez sekretów, bez danych zamówień, ścieżka domowa skrócona do `~`.
pub fn report(input: ReportInput) -> String {
    let mut lines = vec![
        format!("{} {}", crate::APP_NAME, crate::APP_VERSION),
        format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH),
        format!("Katalog danych: {}", shorten_home(input.data_dir)),
        format!(
            "Magazyn poświadczeń: {}",
            match input.credential_store {
                Ok(()) => "OK".to_string(),
                Err(e) => format!("BŁĄD — {e}"),
            }
        ),
        format!("Serwer MCP: {}", input.mcp_status),
        format!("Schemat konfiguracji: v{}", input.config.schema_version),
        format!("Źródła: {}", input.config.sources.len()),
    ];
    for s in &input.config.sources {
        let test = match &s.last_test {
            Some(t) if t.ok => format!("OK ({})", crate::config::iso(t.at)),
            Some(t) => format!("{} — {} ({})", t.code.as_deref().unwrap_or("BŁĄD"), t.message, crate::config::iso(t.at)),
            None => "nie testowano".to_string(),
        };
        lines.push(format!("- {} [{}] {} | ostatni test: {}", s.source_id, s.provider, if s.enabled { "aktywne" } else { "wyłączone" }, test));
    }
    redact(&lines.join("\n"))
}

fn shorten_home(path: &std::path::Path) -> String {
    let full = path.display().to_string();
    match dirs::home_dir() {
        Some(home) => full.replacen(&home.display().to_string(), "~", 1),
        None => full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_registered_secret() {
        register_secret("moj-tajny-token");
        assert_eq!(redact("X-BLToken: moj-tajny-token!"), "X-BLToken: [REDACTED]!");
    }

    #[test]
    fn redacts_token_like_strings_but_keeps_normal_text() {
        let token = "4005-10023-K3O7KPZ1QW9ER8TY7UI6OP5AS4DF3GH2JK1L";
        let redacted = redact(&format!("auth failed for {token}"));
        assert!(!redacted.contains("K3O7KPZ1"), "{redacted}");
        assert!(redacted.contains("auth failed for 4005-10023-"));
        // nazwy narzędzi i zwykłe zdania zostają nietknięte
        let plain = "baselinker__glowny_sklep_2__update_order_status zamowienie 123456789";
        assert_eq!(redact(plain), plain);
    }

    #[test]
    fn report_never_contains_secrets() {
        register_secret("sekret-w-raporcie-1234");
        let mut config = Config::default();
        config.sources.push(crate::config::Source {
            source_id: "sklep".into(),
            provider: "baselinker".into(),
            name: "Sklep".into(),
            enabled: true,
            created_at: 0,
            settings: serde_json::json!({}),
            last_test: Some(crate::config::TestResult {
                ok: false,
                code: Some("AUTH_FAILED".into()),
                message: "bad token sekret-w-raporcie-1234".into(),
                at: 0,
            }),
        });
        let text = report(ReportInput { config: &config, data_dir: std::path::Path::new("/tmp/x"), credential_store: &Ok(()), mcp_status: "gotowy" });
        assert!(!text.contains("sekret-w-raporcie-1234"));
        assert!(text.contains("AUTH_FAILED"));
    }
}
