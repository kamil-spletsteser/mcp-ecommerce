//! Logika komend GUI, niezależna od Tauri (testowalna bez okna). GUI nigdy nie dostaje sekretów z powrotem:
//! wszystkie typy `*View` są bezsekretne z konstrukcji.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::{self, Config, Source, TestResult};
use crate::integrations::{AuthPoll, ErrorCode, Provider, ProviderMeta, Registry, SourceContext, ToolError};
use crate::secrets::{Secret, SecretKey, SecretStore};

const NAME_MAX_CHARS: usize = 60;
const MAX_SOURCES: usize = 20;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    fn new(code: &str, message: impl AsRef<str>) -> Self {
        Self { code: code.into(), message: crate::diagnostics::redact(message.as_ref()) }
    }
}

impl From<ToolError> for CommandError {
    fn from(e: ToolError) -> Self {
        Self { code: e.code.as_str().into(), message: e.message }
    }
}

impl From<config::ConfigError> for CommandError {
    fn from(e: config::ConfigError) -> Self {
        Self::new("CONFIG_ERROR", e.to_string())
    }
}

#[derive(Serialize, Debug)]
pub struct StoreStatus {
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ProviderView {
    #[serde(flatten)]
    pub meta: ProviderMeta,
    /// Sufiksy nazw narzędzi MCP z flagą zapisu — do widoku „Zobacz szczegóły”.
    pub tools: Vec<ToolView>,
}

#[derive(Serialize, Debug)]
pub struct ToolView {
    pub name: &'static str,
    pub read_only: bool,
}

#[derive(Serialize, Debug)]
pub struct StateView {
    pub app_version: &'static str,
    pub providers: Vec<ProviderView>,
    pub sources: Vec<Source>,
    pub credential_store: StoreStatus,
    pub config_error: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ClientSetup {
    pub binary_path: String,
    pub claude_desktop_config_path: &'static str,
    pub claude_desktop_json: String,
}

/// Trwająca autoryzacja w przeglądarce. `device_code` nigdy nie opuszcza Rusta.
struct PendingAuth {
    device_code: Secret,
    interval: Duration,
    deadline: Instant,
}

#[derive(Serialize, Debug)]
pub struct AuthorizationView {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_secs: u64,
}

/// Pola formularza rozdzielone wg `FieldSpec.secret`.
struct FormValues {
    settings: Vec<(String, String)>,
    secrets: Vec<(String, Secret)>,
}

pub struct Service {
    registry: Registry,
    secrets: Arc<dyn SecretStore>,
    data_dir: PathBuf,
    /// Serializuje odczyt-modyfikację-zapis konfiguracji między komendami.
    lock: tokio::sync::Mutex<()>,
    pending_auth: Mutex<HashMap<String, Arc<PendingAuth>>>,
    /// O ile rzadziej odpytywać po `slow_down` (RFC 8628: +5 s). Pole, bo testy nie mogą spać po 5 s.
    pub slow_down_step: Duration,
}

impl Service {
    pub fn new(registry: Registry, secrets: Arc<dyn SecretStore>, data_dir: PathBuf) -> Self {
        Self { registry, secrets, data_dir, lock: Default::default(), pending_auth: Default::default(), slow_down_step: Duration::from_secs(5) }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn credential_store_status(&self) -> Result<(), String> {
        self.secrets.status().map_err(|e| e.to_string())
    }

    pub fn state(&self) -> StateView {
        let (config, config_error) = match config::load(&self.data_dir) {
            Ok(config) => (config, None),
            Err(e) => (Config::default(), Some(e.to_string())),
        };
        let store = self.credential_store_status();
        StateView {
            app_version: crate::APP_VERSION,
            providers: self
                .registry
                .metas()
                .into_iter()
                .map(|meta| {
                    let tools = self.registry.get(meta.id).map(|p| p.tools()).unwrap_or_default();
                    ProviderView { meta, tools: tools.iter().map(|t| ToolView { name: t.name, read_only: t.read_only }).collect() }
                })
                .collect(),
            sources: config.sources,
            credential_store: StoreStatus { ok: store.is_ok(), message: store.err() },
            config_error,
        }
    }

    fn provider(&self, id: &str) -> Result<&Arc<dyn Provider>, CommandError> {
        self.registry.get(id).ok_or_else(|| CommandError::new("PROVIDER_UNAVAILABLE", format!("Provider '{id}' is not available.")))
    }

    fn validate_name(name: &str) -> Result<String, CommandError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > NAME_MAX_CHARS {
            return Err(CommandError::new("VALIDATION_ERROR", format!("Source name must be 1–{NAME_MAX_CHARS} characters.")));
        }
        Ok(name.to_string())
    }

    /// Waliduje wartości formularza względem pól providera. `partial` = edycja (puste pola oznaczają „bez zmian”).
    fn validate_fields(provider: &dyn Provider, fields: &HashMap<String, String>, partial: bool) -> Result<FormValues, CommandError> {
        let meta = provider.meta();
        if let Some(unknown) = fields.keys().find(|k| !meta.fields.iter().any(|f| f.key == k.as_str())) {
            return Err(CommandError::new("VALIDATION_ERROR", format!("Unknown field '{unknown}'.")));
        }
        let mut out = FormValues { settings: vec![], secrets: vec![] };
        for field in &meta.fields {
            match fields.get(field.key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                Some(value) => {
                    // sekret od razu trafia do redaktora — także gdy walidacja go odrzuci i opisze w błędzie
                    let secret = field.secret.then(|| Secret::new(value.to_string()));
                    if value.len() > field.max_len {
                        return Err(CommandError::new("VALIDATION_ERROR", format!("Field '{}' is too long.", field.key)));
                    }
                    provider.validate_field(field.key, value)?;
                    match secret {
                        Some(secret) => out.secrets.push((field.key.to_string(), secret)),
                        None => out.settings.push((field.key.to_string(), value.to_string())),
                    }
                }
                None if field.required && !partial => return Err(CommandError::new("VALIDATION_ERROR", format!("Field '{}' is required.", field.key))),
                None => {}
            }
        }
        Ok(out)
    }

    fn store_error(e: crate::secrets::SecretError) -> CommandError {
        CommandError::new("CREDENTIAL_STORE_ERROR", e.to_string())
    }

    /// Walidacja lokalna → zapis konfiguracji bez sekretów → sekrety do credential store → test połączenia.
    /// Nieudany zapis sekretu wycofuje źródło (żadnego fallbacku do pliku); nieudany test zostawia źródło jako „wymaga uwagi”.
    /// Dla źródeł OAuth ten pierwszy test kończy się bez sieci kodem `CREDENTIAL_UNAVAILABLE` — to stan „konto jeszcze
    /// niepołączone”, z którego GUI prowadzi do `start_authorization`.
    pub async fn add_source(&self, provider_id: &str, name: &str, fields: HashMap<String, String>) -> Result<Source, CommandError> {
        let provider = self.provider(provider_id)?;
        let name = Self::validate_name(name)?;
        let values = Self::validate_fields(provider.as_ref(), &fields, false)?;

        let source = {
            let _guard = self.lock.lock().await;
            let mut config = config::load(&self.data_dir)?;
            if config.sources.len() >= MAX_SOURCES {
                return Err(CommandError::new("VALIDATION_ERROR", format!("At most {MAX_SOURCES} sources are supported.")));
            }
            let source = Source {
                source_id: config::new_source_id(&name, &config.sources),
                provider: provider_id.to_string(),
                name,
                enabled: true,
                created_at: config::now(),
                settings: values.settings.into_iter().map(|(k, v)| (k, serde_json::Value::String(v))).collect::<serde_json::Map<_, _>>().into(),
                last_test: None,
            };
            config.sources.push(source.clone());
            config::save(&self.data_dir, &config)?;
            source
        };

        for (kind, secret) in &values.secrets {
            if let Err(e) = self.secrets.set(&SecretKey::new(provider_id, &source.source_id, kind), secret) {
                let _ = self.remove_source(&source.source_id).await;
                return Err(Self::store_error(e));
            }
        }
        self.test_source(&source.source_id).await
    }

    pub async fn update_source(&self, source_id: &str, name: Option<String>, fields: HashMap<String, String>) -> Result<Source, CommandError> {
        let source = self.find(source_id)?;
        let provider = self.provider(&source.provider)?;
        let values = Self::validate_fields(provider.as_ref(), &fields, true)?;
        let name = name.map(|n| Self::validate_name(&n)).transpose()?;

        let settings_changed = values.settings.iter().any(|(k, v)| source.settings[k.as_str()].as_str() != Some(v.as_str()));
        let credentials_changed = settings_changed || !values.secrets.is_empty();
        for (kind, secret) in &values.secrets {
            self.secrets.set(&SecretKey::new(&source.provider, source_id, kind), secret).map_err(Self::store_error)?;
        }
        if credentials_changed {
            // tokeny OAuth wystawione dla poprzednich danych aplikacji są bezużyteczne — konto trzeba autoryzować od nowa
            self.delete_secrets(&source, provider.token_kinds().iter().copied())?;
        }
        let updated = self
            .modify(source_id, |s| {
                if let Some(name) = name {
                    s.name = name;
                }
                for (key, value) in values.settings {
                    s.settings[key] = value.into();
                }
            })
            .await?;
        if credentials_changed {
            self.test_source(source_id).await
        } else {
            Ok(updated)
        }
    }

    pub async fn set_enabled(&self, source_id: &str, enabled: bool) -> Result<Source, CommandError> {
        self.modify(source_id, |s| s.enabled = enabled).await
    }

    /// Wynik testu (także negatywny) jest zapisywany w konfiguracji w zredagowanej postaci i zwracany w `Source.last_test`.
    pub async fn test_source(&self, source_id: &str) -> Result<Source, CommandError> {
        let source = self.find(source_id)?;
        let provider = self.provider(&source.provider)?;
        let outcome = provider.test_connection(&SourceContext { source: &source, secrets: self.secrets.as_ref() }).await;
        let result = match outcome {
            Ok(message) => TestResult { ok: true, code: None, message, at: config::now() },
            Err(e) => TestResult { ok: false, code: Some(e.code.as_str().into()), message: e.message, at: config::now() },
        };
        self.modify(source_id, |s| s.last_test = Some(result)).await
    }

    // --- Autoryzacja w przeglądarce (OAuth Device Flow) ---

    /// Rozpoczyna autoryzację i zwraca to, co GUI pokazuje użytkownikowi. Ponowne wywołanie unieważnia poprzednią próbę.
    pub async fn start_authorization(&self, source_id: &str) -> Result<AuthorizationView, CommandError> {
        let source = self.find(source_id)?;
        let provider = self.provider(&source.provider)?;
        let auth = provider.begin_authorization(&SourceContext { source: &source, secrets: self.secrets.as_ref() }).await?;
        let view = AuthorizationView { user_code: auth.user_code, verification_uri: auth.verification_uri, expires_in_secs: auth.expires_in_secs };
        let pending = PendingAuth {
            device_code: auth.device_code,
            interval: Duration::from_secs(auth.interval_secs),
            deadline: Instant::now() + Duration::from_secs(auth.expires_in_secs),
        };
        self.pending_auth.lock().unwrap_or_else(|e| e.into_inner()).insert(source_id.to_string(), Arc::new(pending));
        Ok(view)
    }

    pub fn cancel_authorization(&self, source_id: &str) {
        self.pending_auth.lock().unwrap_or_else(|e| e.into_inner()).remove(source_id);
    }

    /// Odpytuje providera, aż użytkownik potwierdzi w przeglądarce, kod wygaśnie albo próba zostanie anulowana/zastąpiona.
    pub async fn finish_authorization(&self, source_id: &str) -> Result<Source, CommandError> {
        let cancelled = || CommandError::new("AUTH_CANCELLED", "Authorization was cancelled.");
        let current = |this: &Self| this.pending_auth.lock().unwrap_or_else(|e| e.into_inner()).get(source_id).cloned();
        let pending = current(self).ok_or_else(cancelled)?;
        let source = self.find(source_id)?;
        let provider = self.provider(&source.provider)?;
        let mut interval = pending.interval;

        let outcome = loop {
            tokio::time::sleep(interval).await;
            // anulowano albo rozpoczęto nową próbę → ta pętla kończy się bez dotykania nowego wpisu
            if !current(self).is_some_and(|now| Arc::ptr_eq(&now, &pending)) {
                return Err(cancelled());
            }
            if Instant::now() >= pending.deadline {
                break Err(CommandError::new("AUTH_EXPIRED", "The authorization code expired."));
            }
            match provider.poll_authorization(&SourceContext { source: &source, secrets: self.secrets.as_ref() }, &pending.device_code).await {
                Ok(AuthPoll::Pending) => {}
                Ok(AuthPoll::SlowDown) => interval += self.slow_down_step,
                Ok(AuthPoll::Done) => break Ok(()),
                // chwilowy błąd sieci nie kończy autoryzacji — użytkownik może wciąż potwierdzać w przeglądarce
                Err(e) if e.code == ErrorCode::UpstreamError => {}
                Err(e) => break Err(e.into()),
            }
        };
        self.cancel_authorization(source_id);
        outcome?;
        self.test_source(source_id).await
    }

    fn delete_secrets<'a>(&self, source: &Source, kinds: impl Iterator<Item = &'a str>) -> Result<(), CommandError> {
        for kind in kinds {
            self.secrets.delete(&SecretKey::new(&source.provider, &source.source_id, kind)).map_err(Self::store_error)?;
        }
        Ok(())
    }

    /// Najpierw sekrety (pola formularza + tokeny OAuth), potem konfiguracja: jeśli credential store odmówi, źródło zostaje
    /// i użytkownik widzi błąd (zamiast osieroconego tokenu, o którym nikt nie wie).
    pub async fn delete_source(&self, source_id: &str) -> Result<(), CommandError> {
        let source = self.find(source_id)?;
        self.cancel_authorization(source_id);
        if let Some(provider) = self.registry.get(&source.provider) {
            let meta = provider.meta();
            let kinds = meta.fields.iter().filter(|f| f.secret).map(|f| f.key).chain(provider.token_kinds().iter().copied());
            self.delete_secrets(&source, kinds)?;
        }
        self.remove_source(source_id).await
    }

    async fn remove_source(&self, source_id: &str) -> Result<(), CommandError> {
        let _guard = self.lock.lock().await;
        let mut config = config::load(&self.data_dir)?;
        config.sources.retain(|s| s.source_id != source_id);
        Ok(config::save(&self.data_dir, &config)?)
    }

    fn find(&self, source_id: &str) -> Result<Source, CommandError> {
        config::load(&self.data_dir)?
            .sources
            .into_iter()
            .find(|s| s.source_id == source_id)
            .ok_or_else(|| ToolError::new(ErrorCode::SourceNotFound, format!("Source '{source_id}' does not exist.")).into())
    }

    async fn modify(&self, source_id: &str, change: impl FnOnce(&mut Source)) -> Result<Source, CommandError> {
        let _guard = self.lock.lock().await;
        let mut config = config::load(&self.data_dir)?;
        let source = config
            .sources
            .iter_mut()
            .find(|s| s.source_id == source_id)
            .ok_or_else(|| CommandError::from(ToolError::new(ErrorCode::SourceNotFound, format!("Source '{source_id}' does not exist."))))?;
        change(source);
        let updated = source.clone();
        config::save(&self.data_dir, &config)?;
        Ok(updated)
    }
}

/// Gotowe do skopiowania fragmenty konfiguracji klientów AI. Wskazują dołączone binarium — bez npx/Node.
pub fn client_setup(exe: &Path) -> ClientSetup {
    const SERVER_NAME: &str = "ecommerce-mcp";
    let path = exe.display().to_string();
    let claude = serde_json::json!({ "mcpServers": { SERVER_NAME: { "command": path, "args": [crate::MCP_ARG] } } });
    ClientSetup {
        claude_desktop_config_path: if cfg!(windows) {
            r"%APPDATA%\Claude\claude_desktop_config.json"
        } else {
            "~/Library/Application Support/Claude/claude_desktop_config.json"
        },
        claude_desktop_json: serde_json::to_string_pretty(&claude).unwrap_or_default(),
        binary_path: path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::baselinker::BaseLinker;
    use crate::secrets::MemoryStore;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "4005-10023-SERVICETOKENSERVICETOKEN0123456789AB";

    struct Fixture {
        service: Service,
        secrets: Arc<MemoryStore>,
        dir: tempfile::TempDir,
        _server: MockServer,
    }

    async fn fixture(api_response: serde_json::Value, store: MemoryStore) -> Fixture {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(api_response)).mount(&server).await;
        let registry = Registry::new(vec![
            Arc::new(BaseLinker::with_client(server.uri(), Duration::from_secs(2), Duration::from_millis(1))),
            Arc::new(crate::integrations::allegro::Allegro::default()),
        ]);
        let secrets = Arc::new(store);
        let dir = tempfile::tempdir().unwrap();
        Fixture { service: Service::new(registry, secrets.clone(), dir.path().join("data")), secrets, dir, _server: server }
    }

    fn token_form() -> HashMap<String, String> {
        HashMap::from([("api_token".to_string(), TOKEN.to_string())])
    }

    fn ok_response() -> serde_json::Value {
        json!({ "status": "SUCCESS", "statuses": [{ "id": 1, "name": "Nowe" }] })
    }

    #[tokio::test]
    async fn add_source_stores_token_only_in_secret_store() {
        let f = fixture(ok_response(), MemoryStore::default()).await;
        let source = f.service.add_source("baselinker", "Główny sklep", token_form()).await.unwrap();
        assert_eq!(source.source_id, "glowny_sklep");
        assert!(source.last_test.as_ref().unwrap().ok);

        let key = SecretKey::new("baselinker", "glowny_sklep", "api_token");
        assert_eq!(f.secrets.get(&key).unwrap().unwrap().expose(), TOKEN);

        // token nie istnieje nigdzie w katalogu danych ani w odpowiedziach dla GUI
        for entry in std::fs::read_dir(f.dir.path().join("data")).unwrap() {
            let content = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(!content.contains(TOKEN) && !content.contains("SERVICETOKEN"), "{content}");
        }
        let responses = format!("{}{}", serde_json::to_string(&source).unwrap(), serde_json::to_string(&f.service.state()).unwrap());
        assert!(!responses.contains("SERVICETOKEN"), "{responses}");
    }

    #[tokio::test]
    async fn failed_test_keeps_source_as_needing_attention() {
        let f =
            fixture(json!({ "status": "ERROR", "error_code": "ERROR_BAD_TOKEN", "error_message": format!("token {TOKEN} invalid") }), MemoryStore::default())
                .await;
        let source = f.service.add_source("baselinker", "Sklep", token_form()).await.unwrap();
        let test = source.last_test.unwrap();
        assert_eq!((test.ok, test.code.as_deref()), (false, Some("AUTH_FAILED")));
        assert!(!test.message.contains("SERVICETOKEN"));
        assert_eq!(f.service.state().sources.len(), 1);
    }

    #[tokio::test]
    async fn unavailable_credential_store_rolls_back_and_writes_no_plaintext() {
        let f = fixture(ok_response(), MemoryStore::unavailable()).await;
        let error = f.service.add_source("baselinker", "Sklep", token_form()).await.unwrap_err();
        assert_eq!(error.code, "CREDENTIAL_STORE_ERROR");
        assert!(f.service.state().sources.is_empty(), "źródło wycofane");
        assert!(!f.service.state().credential_store.ok);
        let config = std::fs::read_to_string(f.dir.path().join("data/config.json")).unwrap();
        assert!(!config.contains("SERVICETOKEN"));
    }

    #[tokio::test]
    async fn disable_update_and_delete_flow() {
        let f = fixture(ok_response(), MemoryStore::default()).await;
        f.service.add_source("baselinker", "Sklep", token_form()).await.unwrap();

        assert!(!f.service.set_enabled("sklep", false).await.unwrap().enabled);

        let renamed = f.service.update_source("sklep", Some("Sklep PL".into()), HashMap::new()).await.unwrap();
        assert_eq!((renamed.name.as_str(), renamed.source_id.as_str()), ("Sklep PL", "sklep"));

        let new_token = "4005-10023-NEWTOKENNEWTOKENNEWTOKEN0123456789AB";
        f.service.update_source("sklep", None, HashMap::from([("api_token".into(), new_token.into())])).await.unwrap();
        let key = SecretKey::new("baselinker", "sklep", "api_token");
        assert_eq!(f.secrets.get(&key).unwrap().unwrap().expose(), new_token);

        f.service.delete_source("sklep").await.unwrap();
        assert!(f.secrets.get(&key).unwrap().is_none(), "usunięcie źródła usuwa token");
        assert!(f.service.state().sources.is_empty());
    }

    #[tokio::test]
    async fn input_validation() {
        let f = fixture(ok_response(), MemoryStore::default()).await;
        let code = |r: Result<Source, CommandError>| r.unwrap_err().code;
        assert_eq!(code(f.service.add_source("shopify", "Konto", HashMap::new()).await), "PROVIDER_UNAVAILABLE");
        assert_eq!(code(f.service.add_source("baselinker", "  ", token_form()).await), "VALIDATION_ERROR");
        assert_eq!(code(f.service.add_source("baselinker", "Sklep", HashMap::new()).await), "VALIDATION_ERROR");
        assert_eq!(code(f.service.add_source("baselinker", "Sklep", HashMap::from([("api_token".into(), "zły token".into())])).await), "VALIDATION_ERROR");
        assert_eq!(code(f.service.add_source("baselinker", "Sklep", HashMap::from([("password".into(), TOKEN.into())])).await), "VALIDATION_ERROR");
        assert_eq!(code(f.service.test_source("nope").await), "SOURCE_NOT_FOUND");
        assert!(!f.dir.path().join("data").exists(), "nieudane próby nie tworzą katalogu danych");
    }

    // --- Allegro: OAuth Device Flow przez Service ---

    const CLIENT_SECRET: &str = "AllegroClientSecretAllegroClientSecret0123456789";

    async fn allegro_fixture() -> (Service, Arc<MemoryStore>, tempfile::TempDir, MockServer) {
        use wiremock::matchers::path;
        let server = MockServer::start().await;
        Mock::given(path("/auth/oauth/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "device-code-SERVICE", "user_code": "xyz-987", "interval": 1, "expires_in": 600,
                "verification_uri_complete": format!("{}/skojarz-aplikacje?code=xyz987", server.uri()),
            })))
            .mount(&server)
            .await;
        Mock::given(path("/me")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "login": "sklep_demo" }))).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let allegro = crate::integrations::allegro::Allegro::with_urls(
            server.uri(),
            server.uri(),
            Duration::from_secs(2),
            Duration::from_millis(1),
            dir.path().join("refresh.lock"),
        );
        let secrets = Arc::new(MemoryStore::default());
        let mut service = Service::new(Registry::new(vec![Arc::new(allegro)]), secrets.clone(), dir.path().join("data"));
        service.slow_down_step = Duration::ZERO;
        (service, secrets, dir, server)
    }

    fn allegro_form() -> HashMap<String, String> {
        HashMap::from([("client_id".to_string(), "0123456789abcdef0123456789abcdef".to_string()), ("client_secret".to_string(), CLIENT_SECRET.to_string())])
    }

    fn token_response(status: u16, body: serde_json::Value) -> Mock {
        Mock::given(wiremock::matchers::path("/auth/oauth/token")).respond_with(ResponseTemplate::new(status).set_body_json(body))
    }

    #[tokio::test]
    async fn allegro_connects_through_device_flow_without_leaking_anything() {
        let (service, secrets, dir, server) = allegro_fixture().await;

        // 1. formularz: Client ID do konfiguracji, Client Secret tylko do credential store; konto jeszcze niepołączone (bez sieci)
        let source = service.add_source("allegro", "Moje Allegro", allegro_form()).await.unwrap();
        assert_eq!(source.settings["client_id"], "0123456789abcdef0123456789abcdef");
        assert_eq!(source.last_test.as_ref().unwrap().code.as_deref(), Some("CREDENTIAL_UNAVAILABLE"));
        assert_eq!(server.received_requests().await.unwrap().len(), 0);

        // 2. start: GUI dostaje kod użytkownika i adres, nigdy device_code
        let view = service.start_authorization("moje_allegro").await.unwrap();
        assert_eq!(view.user_code, "xyz-987");
        assert!(!serde_json::to_string(&view).unwrap().contains("device-code"));

        // 3. polling: slow_down → sukces → test połączenia
        token_response(400, json!({ "error": "slow_down" })).up_to_n_times(1).mount(&server).await;
        token_response(200, json!({ "access_token": "access-token-SERVICE", "refresh_token": "refresh-token-SERVICE" })).mount(&server).await;
        let connected = service.finish_authorization("moje_allegro").await.unwrap();
        let test = connected.last_test.unwrap();
        assert!(test.ok && test.message == "Connected as sklep_demo.", "{test:?}");
        let stored = |kind: &str| secrets.get(&SecretKey::new("allegro", "moje_allegro", kind)).unwrap().map(|s| s.expose().to_string());
        assert_eq!(stored("refresh_token").as_deref(), Some("refresh-token-SERVICE"));

        // 4. nic wrażliwego na dysku ani w odpowiedziach dla GUI
        let exposed = format!("{}{}", std::fs::read_to_string(dir.path().join("data/config.json")).unwrap(), serde_json::to_string(&service.state()).unwrap());
        for secret in [CLIENT_SECRET, "access-token", "refresh-token", "device-code"] {
            assert!(!exposed.contains(secret), "{secret} wyciekł: {exposed}");
        }

        // 5. zmiana danych aplikacji kasuje tokeny (wymusza ponowną autoryzację); usunięcie źródła kasuje wszystko
        let changed = service.update_source("moje_allegro", None, HashMap::from([("client_secret".into(), format!("{CLIENT_SECRET}X"))])).await.unwrap();
        assert_eq!(changed.last_test.unwrap().code.as_deref(), Some("CREDENTIAL_UNAVAILABLE"));
        assert_eq!((stored("access_token"), stored("refresh_token")), (None, None));
        service.delete_source("moje_allegro").await.unwrap();
        assert_eq!(stored("client_secret"), None);
    }

    #[tokio::test]
    async fn allegro_authorization_can_be_cancelled_or_denied() {
        let (service, _secrets, _dir, server) = allegro_fixture().await;
        service.add_source("allegro", "Moje Allegro", allegro_form()).await.unwrap();
        assert_eq!(service.finish_authorization("moje_allegro").await.unwrap_err().code, "AUTH_CANCELLED", "bez rozpoczętej autoryzacji");

        service.start_authorization("moje_allegro").await.unwrap();
        service.cancel_authorization("moje_allegro");
        assert_eq!(service.finish_authorization("moje_allegro").await.unwrap_err().code, "AUTH_CANCELLED");
        assert!(!server.received_requests().await.unwrap().iter().any(|r| r.url.path() == "/auth/oauth/token"), "anulowana próba nie odpytuje Allegro");

        service.start_authorization("moje_allegro").await.unwrap();
        token_response(400, json!({ "error": "access_denied" })).mount(&server).await;
        assert_eq!(service.finish_authorization("moje_allegro").await.unwrap_err().code, "AUTH_FAILED");
    }

    #[test]
    fn client_setup_points_at_bundled_binary() {
        let setup = client_setup(Path::new("/Applications/E-commerce MCP.app/Contents/MacOS/ecommerce-mcp"));
        let claude: serde_json::Value = serde_json::from_str(&setup.claude_desktop_json).unwrap();
        assert_eq!(claude["mcpServers"]["ecommerce-mcp"]["command"], "/Applications/E-commerce MCP.app/Contents/MacOS/ecommerce-mcp");
        assert_eq!(claude["mcpServers"]["ecommerce-mcp"]["args"], json!(["mcp"]));
        assert!(!setup.claude_desktop_json.contains("npx") && !setup.claude_desktop_json.contains("node"));
    }
}
