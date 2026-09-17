//! E-commerce MCP — wspólny rdzeń GUI (Tauri) i serwera MCP (stdio).
//! To samo binarium: bez argumentów uruchamia GUI, z argumentem `mcp` serwer MCP.

pub mod app;
pub mod config;
pub mod diagnostics;
pub mod integrations;
pub mod mcp;
pub mod secrets;

pub const APP_NAME: &str = "E-commerce MCP";
/// Identyfikator bundla: katalog danych aplikacji i namespace wpisów w credential store.
pub const APP_ID: &str = "com.lampartoms.ecommerce-mcp";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Argument przełączający binarium w tryb serwera MCP.
pub const MCP_ARG: &str = "mcp";
