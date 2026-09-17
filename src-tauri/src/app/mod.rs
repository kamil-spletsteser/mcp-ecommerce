//! Warstwa Tauri: cienkie komendy nad `Service`. GUI rozmawia z Rustem wyłącznie przez te komendy —
//! nie ma dostępu do plików konfiguracji ani do credential store.

pub mod plugin;
pub mod selfcheck;
pub mod service;

use std::collections::HashMap;

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::config::Source;
use crate::integrations::Registry;
use selfcheck::McpCheck;
use service::{AuthorizationView, ClientSetup, CommandError, Service, StateView};

type CommandResult<T> = Result<T, CommandError>;

fn current_exe() -> CommandResult<std::path::PathBuf> {
    std::env::current_exe().map_err(|e| CommandError { code: "INTERNAL_ERROR".into(), message: format!("cannot locate application binary: {e}") })
}

#[tauri::command]
async fn get_state(service: State<'_, Service>) -> CommandResult<StateView> {
    Ok(service.state())
}

#[tauri::command]
async fn add_source(service: State<'_, Service>, provider: String, name: String, fields: HashMap<String, String>) -> CommandResult<Source> {
    service.add_source(&provider, &name, fields).await
}

#[tauri::command]
async fn update_source(service: State<'_, Service>, source_id: String, name: Option<String>, fields: HashMap<String, String>) -> CommandResult<Source> {
    service.update_source(&source_id, name, fields).await
}

/// Rozpoczyna autoryzację i od razu otwiera stronę potwierdzenia w domyślnej przeglądarce. Adres pochodzi od providera
/// (który sprawdza, że prowadzi do jego serwisu logowania) — webview podaje wyłącznie `source_id`.
#[tauri::command]
async fn start_authorization(app: tauri::AppHandle, service: State<'_, Service>, source_id: String) -> CommandResult<AuthorizationView> {
    let view = service.start_authorization(&source_id).await?;
    let _ = app.opener().open_url(&view.verification_uri, None::<&str>); // adres jest też widoczny w GUI — brak przeglądarki nie blokuje
    Ok(view)
}

/// Czeka (także kilka minut), aż użytkownik potwierdzi dostęp w przeglądarce.
#[tauri::command]
async fn finish_authorization(service: State<'_, Service>, source_id: String) -> CommandResult<Source> {
    service.finish_authorization(&source_id).await
}

#[tauri::command]
async fn cancel_authorization(service: State<'_, Service>, source_id: String) -> CommandResult<()> {
    service.cancel_authorization(&source_id);
    Ok(())
}

#[tauri::command]
async fn set_source_enabled(service: State<'_, Service>, source_id: String, enabled: bool) -> CommandResult<Source> {
    service.set_enabled(&source_id, enabled).await
}

#[tauri::command]
async fn test_source(service: State<'_, Service>, source_id: String) -> CommandResult<Source> {
    service.test_source(&source_id).await
}

#[tauri::command]
async fn delete_source(service: State<'_, Service>, source_id: String) -> CommandResult<()> {
    service.delete_source(&source_id).await
}

#[tauri::command]
async fn check_mcp() -> CommandResult<McpCheck> {
    Ok(selfcheck::check_mcp(&current_exe()?).await)
}

#[tauri::command]
async fn client_setup() -> CommandResult<ClientSetup> {
    Ok(service::client_setup(&current_exe()?))
}

/// Zapisuje plugin do katalogu Pobrane i pokazuje go w Finderze/Eksploratorze. Instalację w Claude wykonuje użytkownik
/// („add plugin” → wskazanie pliku) — aplikacja nie modyfikuje konfiguracji klientów AI.
#[tauri::command]
async fn export_plugin(app: tauri::AppHandle) -> CommandResult<String> {
    let internal = |message: String| CommandError { code: "EXPORT_FAILED".into(), message };
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or_else(|| internal("no Downloads directory".into()))?;
    let path = plugin::export(&current_exe()?, &dir).map_err(internal)?;
    let _ = app.opener().reveal_item_in_dir(&path); // samo pokazanie pliku jest opcjonalne
    Ok(path.display().to_string())
}

#[tauri::command]
async fn diagnostic_report(service: State<'_, Service>) -> CommandResult<String> {
    let check = selfcheck::check_mcp(&current_exe()?).await;
    let mcp_status = match &check.error {
        None => format!(
            "OK (v{}, protokół {}, narzędzi: {})",
            check.server_version.unwrap_or_default(),
            check.protocol_version.unwrap_or_default(),
            check.tools.len()
        ),
        Some(error) => format!("BŁĄD — {error}"),
    };
    let state = service.state();
    let config = crate::config::Config { schema_version: crate::config::SCHEMA_VERSION, sources: state.sources };
    Ok(crate::diagnostics::report(crate::diagnostics::ReportInput {
        config: &config,
        data_dir: service.data_dir(),
        credential_store: &service.credential_store_status(),
        mcp_status: &mcp_status,
    }))
}

#[tauri::command]
async fn open_data_dir(app: tauri::AppHandle, service: State<'_, Service>) -> CommandResult<()> {
    let dir = service.data_dir();
    if !dir.exists() {
        // katalog powstaje dopiero przy pierwszym zapisie — nie tworzymy go tylko po to, żeby go pokazać
        return Err(CommandError { code: "DATA_DIR_MISSING".into(), message: "Data directory does not exist yet.".into() });
    }
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| CommandError { code: "INTERNAL_ERROR".into(), message: e.to_string() })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Service::new(Registry::default(), crate::secrets::default_store(), crate::config::data_dir()))
        .invoke_handler(tauri::generate_handler![
            get_state,
            add_source,
            update_source,
            start_authorization,
            finish_authorization,
            cancel_authorization,
            set_source_enabled,
            test_source,
            delete_source,
            check_mcp,
            client_setup,
            export_plugin,
            diagnostic_report,
            open_data_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running E-commerce MCP");
}
