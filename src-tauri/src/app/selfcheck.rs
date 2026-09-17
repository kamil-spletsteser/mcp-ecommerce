//! „Sprawdź konfigurację”: uruchamia to samo binarium w trybie `mcp` dokładnie tak, jak zrobi to klient AI,
//! wykonuje handshake MCP + `tools/list` i kończy proces. Nie wywołuje żadnego narzędzia, więc nie dotyka sekretów.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Minimalny klient MCP po stdio (JSON-RPC, jedna wiadomość na linię). Używany przez self-check i testy E2E.
pub struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl StdioSession {
    pub fn spawn(exe: &Path, envs: &[(&str, &str)]) -> Result<Self, String> {
        let mut command = Command::new(exe);
        command.arg(crate::MCP_ARG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(|e| format!("cannot start MCP server process: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin pipe")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no stdout pipe")?).lines();
        Ok(Self { child, stdin, stdout, next_id: 1 })
    }

    async fn send(&mut self, message: Value) -> Result<(), String> {
        let line = format!("{message}\n");
        self.stdin.write_all(line.as_bytes()).await.map_err(|e| format!("MCP server closed its input: {e}"))?;
        self.stdin.flush().await.map_err(|e| e.to_string())
    }

    pub async fn notify(&mut self, method: &str) -> Result<(), String> {
        self.send(json!({ "jsonrpc": "2.0", "method": method })).await
    }

    /// Zwraca `result` odpowiedzi; błąd JSON-RPC, EOF lub timeout → Err.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        let wait = async {
            loop {
                let line = self.stdout.next_line().await.map_err(|e| e.to_string())?.ok_or("MCP server exited before responding")?;
                // stdout należy do protokołu — każda linia musi być poprawnym JSON-RPC
                let message: Value = serde_json::from_str(&line).map_err(|_| "MCP server wrote non-protocol output to stdout".to_string())?;
                if message["id"] == id {
                    return match message.get("error") {
                        Some(error) => Err(format!("MCP error: {}", error["message"].as_str().unwrap_or("unknown"))),
                        None => Ok(message["result"].clone()),
                    };
                }
            }
        };
        tokio::time::timeout(TIMEOUT, wait).await.map_err(|_| format!("MCP server did not answer '{method}' in time"))?
    }

    pub async fn initialize(&mut self) -> Result<Value, String> {
        let result = self
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "ecommerce-mcp-selfcheck", "version": crate::APP_VERSION } }),
            )
            .await?;
        self.notify("notifications/initialized").await?;
        Ok(result)
    }

    /// Zamyka stdin (sygnał końca sesji) i czeka na kontrolowane wyjście; po 3 s dobija proces.
    pub async fn shutdown(mut self) -> Option<i32> {
        drop(self.stdin);
        match tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await {
            Ok(status) => status.ok().and_then(|s| s.code()),
            Err(_) => {
                let _ = self.child.kill().await;
                None
            }
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct McpCheck {
    pub ok: bool,
    pub server_version: Option<String>,
    pub protocol_version: Option<String>,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

pub async fn check_mcp(exe: &Path) -> McpCheck {
    let run = async {
        let mut session = StdioSession::spawn(exe, &[])?;
        let info = session.initialize().await?;
        let listed = session.request("tools/list", json!({})).await?;
        session.shutdown().await;
        let tools = listed["tools"].as_array().map(|t| t.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect()).unwrap_or_default();
        Ok::<_, String>((info, tools))
    };
    match run.await {
        Ok((info, tools)) => McpCheck {
            ok: true,
            server_version: info["serverInfo"]["version"].as_str().map(String::from),
            protocol_version: info["protocolVersion"].as_str().map(String::from),
            tools,
            error: None,
        },
        Err(error) => McpCheck { ok: false, server_version: None, protocol_version: None, tools: vec![], error: Some(crate::diagnostics::redact(&error)) },
    }
}
