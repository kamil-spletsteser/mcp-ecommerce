//! Lokalny serwer MCP po stdio (oficjalny SDK `rmcp`). Bez portów sieciowych.
//!
//! Model nazw: `<provider>__<source_id>__<tool>`, np. `baselinker__glowny_sklep__list_orders`.
//! Lista narzędzi jest dynamiczna — konfiguracja czytana przy każdym `tools/list` i `tools/call`,
//! więc wyłączenie źródła w GUI działa od najbliższego odświeżenia listy / wywołania.
//! Serwer jest bezstanowy; sekret pobierany z credential store dopiero przy wywołaniu narzędzia.
//! `config.json` jest dla tego procesu tylko do odczytu; do credential store zapisuje wyłącznie provider OAuth
//! (odświeżone tokeny Allegro).

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};

use crate::config::{self, Source};
use crate::integrations::{parse_tool_name, tool_name, ErrorCode, Registry, SourceContext, ToolError};
use crate::secrets::SecretStore;

/// Zawsze dostępne narzędzie diagnostyczne: mapuje nazwy źródeł użytkownika na prefiksy narzędzi.
pub const LIST_SOURCES_TOOL: &str = "ecommerce_mcp_list_sources";

const INSTRUCTIONS: &str = "E-commerce MCP gives access to the user's own e-commerce accounts connected in the E-commerce MCP desktop app. \
Tool names follow <provider>__<source_id>__<tool>; call ecommerce_mcp_list_sources to see which source_id belongs to which shop. \
Tools whose description starts with WRITE change live shop data — confirm with the user before calling them. \
Errors are returned as {\"error\":{\"code\",\"message\"}} with codes such as AUTH_FAILED, RATE_LIMITED, SOURCE_DISABLED, VALIDATION_ERROR.";

pub struct McpServer {
    registry: Registry,
    secrets: Arc<dyn SecretStore>,
    data_dir: PathBuf,
}

impl McpServer {
    pub fn new(registry: Registry, secrets: Arc<dyn SecretStore>, data_dir: PathBuf) -> Self {
        Self { registry, secrets, data_dir }
    }

    fn sources(&self) -> Result<Vec<Source>, ToolError> {
        config::load(&self.data_dir)
            .map(|c| c.sources)
            .map_err(|e| ToolError::new(ErrorCode::SourceNotFound, format!("E-commerce MCP configuration could not be read: {e}")))
    }

    /// Tylko źródła aktywne z dostępnym providerem rejestrują narzędzia.
    pub fn tools(&self) -> Result<Vec<Tool>, ToolError> {
        let mut tools = vec![Tool::new(
            LIST_SOURCES_TOOL,
            "List the e-commerce sources configured in the E-commerce MCP app: source_id, user-given name, provider and whether it is enabled. Never returns credentials.",
            object(json!({ "type": "object", "properties": {}, "additionalProperties": false })),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).open_world(false))];

        for source in self.sources()?.iter().filter(|s| s.enabled) {
            let Some(provider) = self.registry.get(&source.provider) else {
                continue;
            };
            for def in provider.tools() {
                let description = format!("[{} · {}] {}", provider.meta().name, source.name, def.description);
                let annotations = ToolAnnotations::new().read_only(def.read_only).destructive(false).open_world(true);
                tools.push(
                    Tool::new(tool_name(&source.provider, &source.source_id, def.name), description, object(def.input_schema)).with_annotations(annotations),
                );
            }
        }
        Ok(tools)
    }

    pub async fn call(&self, name: &str, args: Value) -> Result<Value, ToolError> {
        if name == LIST_SOURCES_TOOL {
            let sources: Vec<Value> = self
                .sources()?
                .iter()
                .map(|s| json!({ "source_id": s.source_id, "name": s.name, "provider": s.provider, "enabled": s.enabled, "tool_prefix": format!("{}__{}__", s.provider, s.source_id) }))
                .collect();
            return Ok(json!({ "sources": sources }));
        }

        let (provider_id, source_id, tool) = parse_tool_name(name).ok_or_else(|| ToolError::validation(format!("Unknown tool '{name}'.")))?;
        let sources = self.sources()?;
        let source = sources
            .iter()
            .find(|s| s.source_id == source_id && s.provider == provider_id)
            .ok_or_else(|| ToolError::new(ErrorCode::SourceNotFound, format!("Source '{source_id}' does not exist (it may have been removed in the app).")))?;
        if !source.enabled {
            return Err(ToolError::new(ErrorCode::SourceDisabled, format!("Source '{source_id}' is disabled in the E-commerce MCP app.")));
        }
        let provider =
            self.registry.get(provider_id).ok_or_else(|| ToolError::new(ErrorCode::SourceNotFound, format!("Provider '{provider_id}' is not available.")))?;
        if !provider.tools().iter().any(|t| t.name == tool) {
            return Err(ToolError::validation(format!("Unknown tool '{name}'.")));
        }
        provider.call_tool(&SourceContext { source, secrets: self.secrets.as_ref() }, tool, &args).await
    }
}

fn object(value: Value) -> Arc<serde_json::Map<String, Value>> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

pub fn error_payload(error: &ToolError) -> Value {
    json!({ "error": { "code": error.code.as_str(), "message": error.message } })
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ecommerce-mcp", crate::APP_VERSION).with_title(crate::APP_NAME))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        match self.tools() {
            Ok(tools) => Ok(ListToolsResult::with_all_items(tools)),
            Err(e) => Err(ErrorData::internal_error(e.message, None)),
        }
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = match self.call(&request.name, args).await {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => {
                crate::diagnostics::log(&format!("tool {} failed: {} — {}", request.name, error.code.as_str(), error.message));
                CallToolResult::structured_error(error_payload(&error))
            }
        };
        Ok(result.into())
    }
}

/// Punkt wejścia `ecommerce-mcp mcp`. Kończy się, gdy klient zamknie stdin. Zwraca kod wyjścia procesu.
pub fn run_stdio() -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            crate::diagnostics::log(&format!("cannot start async runtime: {e}"));
            return 1;
        }
    };
    runtime.block_on(async {
        let server = McpServer::new(Registry::default(), crate::secrets::default_store(), config::data_dir());
        match server.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
                0
            }
            Err(e) => {
                crate::diagnostics::log(&format!("MCP session failed to start: {e}"));
                1
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::secrets::MemoryStore;

    fn server_with(sources: Vec<Source>) -> (McpServer, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        config::save(tmp.path(), &Config { schema_version: config::SCHEMA_VERSION, sources }).unwrap();
        (McpServer::new(Registry::default(), Arc::new(MemoryStore::default()), tmp.path().to_path_buf()), tmp)
    }

    fn source(id: &str, provider: &str, enabled: bool) -> Source {
        Source { source_id: id.into(), provider: provider.into(), name: id.to_uppercase(), enabled, created_at: 0, settings: json!({}), last_test: None }
    }

    fn names(server: &McpServer) -> Vec<String> {
        server.tools().unwrap().iter().map(|t| t.name.to_string()).collect()
    }

    #[test]
    fn tool_list_is_dynamic() {
        let (server, _tmp) =
            server_with(vec![source("a", "baselinker", true), source("b", "baselinker", false), source("c", "allegro", true), source("d", "shopify", true)]);
        let names = names(&server);
        assert!(names.contains(&LIST_SOURCES_TOOL.to_string()));
        assert!(names.contains(&"baselinker__a__list_orders".to_string()));
        assert!(names.contains(&"baselinker__a__update_order_status".to_string()));
        assert!(!names.iter().any(|n| n.contains("__b__")), "wyłączone źródło nie rejestruje narzędzi");
        assert!(names.contains(&"allegro__c__list_orders".to_string()));
        assert!(!names.iter().any(|n| n.starts_with("shopify")), "źródło nieznanego providera nie rejestruje narzędzi");
        assert!(names.iter().all(|n| n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')));
    }

    #[test]
    fn no_sources_means_only_the_builtin_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let server = McpServer::new(Registry::default(), Arc::new(MemoryStore::default()), tmp.path().join("missing"));
        assert_eq!(names(&server), vec![LIST_SOURCES_TOOL]);
    }

    #[tokio::test]
    async fn source_errors_are_normalized() {
        let (server, _tmp) = server_with(vec![source("a", "baselinker", true), source("b", "baselinker", false)]);
        let code = |r: Result<Value, ToolError>| r.unwrap_err().code;
        assert_eq!(code(server.call("baselinker__zzz__list_orders", json!({})).await), ErrorCode::SourceNotFound);
        assert_eq!(code(server.call("baselinker__b__list_orders", json!({})).await), ErrorCode::SourceDisabled);
        assert_eq!(code(server.call("baselinker__a__drop_database", json!({})).await), ErrorCode::ValidationError);
        assert_eq!(code(server.call("nonsense", json!({})).await), ErrorCode::ValidationError);
        // źródło aktywne, ale bez tokenu w credential store
        assert_eq!(code(server.call("baselinker__a__get_order_statuses", json!({})).await), ErrorCode::CredentialUnavailable);
        // walidacja wejścia następuje przed jakimkolwiek dostępem do sieci/sekretów
        assert_eq!(code(server.call("baselinker__a__get_order", json!({"order_id": "12"})).await), ErrorCode::ValidationError);
    }

    #[tokio::test]
    async fn list_sources_never_exposes_secrets() {
        let (server, _tmp) = server_with(vec![source("a", "baselinker", true)]);
        let out = server.call(LIST_SOURCES_TOOL, Value::Null).await.unwrap();
        assert_eq!(out["sources"][0]["tool_prefix"], "baselinker__a__");
        assert!(!out.to_string().to_lowercase().contains("token"));
    }
}
