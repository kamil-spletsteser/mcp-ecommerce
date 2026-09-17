//! Bezsekretna konfiguracja na dysku: wersjonowany JSON, migracje, atomowy zapis.
//!
//! Lokalizacja: `<data_local_dir>/com.lampartoms.ecommerce-mcp/config.json`
//! (macOS: `~/Library/Application Support`, Windows: `%LOCALAPPDATA%` — lokalnie, bo
//! wpisy Credential Managera też nie roamują; konfiguracja bez tokenów na innym komputerze byłaby bezużyteczna).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "config.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    pub schema_version: u32,
    #[serde(default)]
    pub sources: Vec<Source>,
}

impl Default for Config {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, sources: Vec::new() }
    }
}

/// Skonfigurowane źródło. Celowo nie ma tu żadnego pola na sekret — tokeny żyją wyłącznie w credential store.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    /// Stabilny identyfikator (slug) — część nazw narzędzi MCP i klucza w credential store. Nie zmienia się przy zmianie nazwy.
    pub source_id: String,
    pub provider: String,
    pub name: String,
    pub enabled: bool,
    /// Unix timestamp (s).
    pub created_at: u64,
    /// Niesekretne ustawienia specyficzne dla providera.
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub last_test: Option<TestResult>,
}

/// Ostatni, zredagowany wynik testu połączenia.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TestResult {
    pub ok: bool,
    pub code: Option<String>,
    pub message: String,
    pub at: u64,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Invalid(String),
    /// Plik zapisany przez nowszą wersję aplikacji — nie ruszamy go.
    TooNew(u32),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "config I/O error: {e}"),
            Self::Invalid(e) => write!(f, "invalid config file: {e}"),
            Self::TooNew(v) => write!(f, "config schema v{v} is newer than supported v{SCHEMA_VERSION}"),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub fn data_dir() -> PathBuf {
    // Nadpisanie katalogu tylko w buildach debug (testy E2E); release zawsze używa katalogu systemowego.
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("ECOMMERCE_MCP_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join(crate::APP_ID)
}

/// Brak pliku = pusta konfiguracja (katalog powstaje dopiero przy pierwszym zapisie).
pub fn load(dir: &Path) -> Result<Config, ConfigError> {
    let raw = match std::fs::read_to_string(dir.join(FILE_NAME)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.into()),
    };
    let value: Value = serde_json::from_str(&raw).map_err(|e| ConfigError::Invalid(e.to_string()))?;
    serde_json::from_value(migrate(value)?).map_err(|e| ConfigError::Invalid(e.to_string()))
}

/// Atomowo: plik tymczasowy w tym samym katalogu + fsync + rename.
pub fn save(dir: &Path, config: &Config) -> Result<(), ConfigError> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(config).map_err(|e| ConfigError::Invalid(e.to_string()))?;
    let tmp = dir.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        std::fs::rename(&tmp, dir.join(FILE_NAME))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Migracje krok po kroku: vN → vN+1, aż do `SCHEMA_VERSION`.
fn migrate(mut value: Value) -> Result<Value, ConfigError> {
    loop {
        let version = value.get("schema_version").and_then(Value::as_u64).unwrap_or(0) as u32;
        match version {
            SCHEMA_VERSION => return Ok(value),
            0 => value = migrate_v0_to_v1(value),
            v => return Err(ConfigError::TooNew(v)),
        }
    }
}

/// v0 (wczesne buildy): brak `schema_version`, źródła bez flagi `enabled`.
fn migrate_v0_to_v1(mut value: Value) -> Value {
    if let Some(sources) = value.get_mut("sources").and_then(Value::as_array_mut) {
        for source in sources.iter_mut().filter_map(Value::as_object_mut) {
            source.entry("enabled").or_insert(Value::Bool(true));
        }
    }
    if let Some(root) = value.as_object_mut() {
        root.insert("schema_version".into(), 1.into());
    }
    value
}

/// „Główny sklep” → `glowny_sklep`; unikalne względem istniejących źródeł.
/// Tylko `[a-z0-9_]`, bez podwójnych podkreśleń (`__` rozdziela segmenty nazw narzędzi MCP), maks. 20 znaków.
pub fn new_source_id(name: &str, existing: &[Source]) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        let mapped = match c {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        };
        if mapped != '_' || !(slug.is_empty() || slug.ends_with('_')) {
            slug.push(mapped);
        }
    }
    let slug: String = slug.trim_end_matches('_').chars().take(20).collect();
    let slug = slug.trim_end_matches('_');
    let base = if slug.is_empty() { "zrodlo" } else { slug };
    let taken = |id: &str| existing.iter().any(|s| s.source_id == id);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}_{n}")).find(|id| !taken(id)).expect("unbounded range")
}

/// Unix timestamp → `2026-01-31T12:00:00Z` (raport diagnostyczny).
pub fn iso(timestamp: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp(timestamp as i64)
        .ok()
        .and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok())
        .unwrap_or_else(|| timestamp.to_string())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str) -> Source {
        Source {
            source_id: id.into(),
            provider: "baselinker".into(),
            name: "Sklep".into(),
            enabled: true,
            created_at: 1,
            settings: serde_json::json!({}),
            last_test: None,
        }
    }

    #[test]
    fn missing_file_is_empty_config_and_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("not-yet");
        assert_eq!(load(&dir).unwrap(), Config::default());
        assert!(!dir.exists(), "load nie może tworzyć katalogu danych");
    }

    #[test]
    fn save_then_load_roundtrip_leaves_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        let config = Config { schema_version: SCHEMA_VERSION, sources: vec![source("a")] };
        save(&dir, &config).unwrap();
        save(&dir, &config).unwrap(); // nadpisanie istniejącego pliku
        assert_eq!(load(&dir).unwrap(), config);
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, vec![std::ffi::OsString::from(FILE_NAME)]);
    }

    #[test]
    fn migrates_v0_and_rejects_newer_schema() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE_NAME), r#"{"sources":[{"source_id":"a","provider":"baselinker","name":"A","created_at":5}]}"#).unwrap();
        let config = load(tmp.path()).unwrap();
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert!(config.sources[0].enabled);

        std::fs::write(tmp.path().join(FILE_NAME), r#"{"schema_version":99,"sources":[]}"#).unwrap();
        assert!(matches!(load(tmp.path()), Err(ConfigError::TooNew(99))));
    }

    #[test]
    fn corrupted_file_is_an_error_not_a_silent_reset() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(FILE_NAME), "{oops").unwrap();
        assert!(matches!(load(tmp.path()), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn source_ids_are_slugs_and_unique() {
        assert_eq!(new_source_id("Główny sklep", &[]), "glowny_sklep");
        assert_eq!(new_source_id("  Żółć -- & Łódź!  ", &[]), "zolc_lodz");
        assert_eq!(new_source_id("!!!", &[]), "zrodlo");
        assert_eq!(new_source_id("Główny sklep", &[source("glowny_sklep")]), "glowny_sklep_2");
        let long = new_source_id("Bardzo długa nazwa mojego sklepu internetowego", &[]);
        assert!(long.len() <= 20 && !long.contains("__") && !long.ends_with('_'), "{long}");
    }

    #[test]
    fn serialized_config_has_no_secret_fields() {
        let json = serde_json::to_string(&Config { schema_version: 1, sources: vec![source("a")] }).unwrap();
        for forbidden in ["token", "secret", "password"] {
            assert!(!json.to_lowercase().contains(forbidden), "{json}");
        }
    }
}
