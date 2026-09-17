// Jedyny styk GUI z Rustem: komendy Tauri. Żaden typ tutaj nie zawiera sekretu — tokeny płyną tylko w stronę Rusta.
import { invoke } from "@tauri-apps/api/core";

export interface TestResult {
  ok: boolean;
  code: string | null;
  message: string;
  at: number;
}

export interface Source {
  source_id: string;
  provider: string;
  name: string;
  enabled: boolean;
  created_at: number;
  /** Niesekretne ustawienia providera (np. Client ID aplikacji Allegro). */
  settings: Record<string, string>;
  last_test: TestResult | null;
}

export interface Provider {
  id: string;
  name: string;
  auth: "fields" | "oauth_device";
  fields: { key: string; secret: boolean; required: boolean; max_len: number }[];
  capabilities: { label_key: string; write: boolean; tools: string[] }[];
  tools: { name: string; read_only: boolean }[];
}

export interface AppState {
  app_version: string;
  providers: Provider[];
  sources: Source[];
  credential_store: { ok: boolean; message: string | null };
  config_error: string | null;
}

export interface McpCheck {
  ok: boolean;
  server_version: string | null;
  protocol_version: string | null;
  tools: string[];
  error: string | null;
}

export interface ClientSetup {
  binary_path: string;
  claude_desktop_config_path: string;
  claude_desktop_json: string;
}

export interface AuthorizationView {
  user_code: string;
  verification_uri: string;
  expires_in_secs: number;
}

export interface CommandError {
  code: string;
  message: string;
}

export const api = {
  getState: () => invoke<AppState>("get_state"),
  addSource: (provider: string, name: string, fields: Record<string, string>) => invoke<Source>("add_source", { provider, name, fields }),
  updateSource: (sourceId: string, name: string | null, fields: Record<string, string>) => invoke<Source>("update_source", { sourceId, name, fields }),
  /** Rozpoczyna autoryzację w przeglądarce (Rust sam otwiera stronę) i zwraca kod do pokazania użytkownikowi. */
  startAuthorization: (sourceId: string) => invoke<AuthorizationView>("start_authorization", { sourceId }),
  /** Rozwiązuje się dopiero, gdy użytkownik potwierdzi dostęp, kod wygaśnie albo próba zostanie anulowana. */
  finishAuthorization: (sourceId: string) => invoke<Source>("finish_authorization", { sourceId }),
  cancelAuthorization: (sourceId: string) => invoke<void>("cancel_authorization", { sourceId }),
  setSourceEnabled: (sourceId: string, enabled: boolean) => invoke<Source>("set_source_enabled", { sourceId, enabled }),
  testSource: (sourceId: string) => invoke<Source>("test_source", { sourceId }),
  deleteSource: (sourceId: string) => invoke<void>("delete_source", { sourceId }),
  checkMcp: () => invoke<McpCheck>("check_mcp"),
  clientSetup: () => invoke<ClientSetup>("client_setup"),
  exportPlugin: () => invoke<string>("export_plugin"),
  diagnosticReport: () => invoke<string>("diagnostic_report"),
  openDataDir: () => invoke<void>("open_data_dir"),
};

export const toolName = (source: Source, tool: string) => `${source.provider}__${source.source_id}__${tool}`;
