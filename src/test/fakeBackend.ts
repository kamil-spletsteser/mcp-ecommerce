// Udawany backend Tauri w pamięci — wspólny dla testów UI (Vitest) i podglądu w przeglądarce (src/test/preview.html).
// Jak prawdziwy backend: przyjmuje sekrety, ale nigdy ich nie odsyła.
import type { AppState, AuthorizationView, ClientSetup, McpCheck, Source } from "../api";

const providers: AppState["providers"] = [
  {
    id: "baselinker",
    name: "BaseLinker",
    auth: "fields",
    fields: [{ key: "api_token", secret: true, required: true, max_len: 300 }],
    capabilities: [
      { label_key: "cap.orders.read", write: false, tools: ["list_orders", "get_order"] },
      { label_key: "cap.statuses.read", write: false, tools: ["get_order_statuses"] },
      { label_key: "cap.products.read", write: false, tools: ["list_inventories", "list_products"] },
      { label_key: "cap.warehouses.read", write: false, tools: ["list_warehouses"] },
      { label_key: "cap.orders.update_status", write: true, tools: ["update_order_status"] },
      { label_key: "cap.orders.add_note", write: true, tools: ["add_order_note"] },
    ],
    tools: ["get_order_statuses", "list_orders", "get_order", "list_inventories", "list_warehouses", "list_products"]
      .map((name) => ({ name, read_only: true }))
      .concat([
        { name: "update_order_status", read_only: false },
        { name: "add_order_note", read_only: false },
      ]),
  },
  {
    id: "allegro",
    name: "Allegro",
    auth: "oauth_device",
    fields: [
      { key: "client_id", secret: false, required: true, max_len: 100 },
      { key: "client_secret", secret: true, required: true, max_len: 200 },
    ],
    capabilities: [
      { label_key: "cap.orders.read", write: false, tools: ["list_orders", "get_order"] },
      { label_key: "cap.offers.read", write: false, tools: ["list_offers", "get_offer"] },
      { label_key: "cap.account.read", write: false, tools: ["get_account"] },
    ],
    tools: ["get_account", "list_orders", "get_order", "list_offers", "get_offer"].map((name) => ({ name, read_only: true })),
  },
];

const slug = (name: string) => name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/ł/g, "l").replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
const NOT_CONNECTED = { ok: false, code: "CREDENTIAL_UNAVAILABLE", message: "This Allegro account is not connected yet.", at: 1750000000 };

export const backend = {
  sources: [] as Source[],
  calls: [] as { cmd: string; args: Record<string, unknown> }[],
  testOk: true,
  /** Trwająca autoryzacja: test rozstrzyga ją ręcznie (`authorization.resolve()` / `.reject(...)`), podgląd — po `autoApproveMs`. */
  authorization: null as { resolve: () => void; reject: (error: unknown) => void } | null,
  autoApproveMs: null as number | null,

  reset() {
    this.sources = [];
    this.calls = [];
    this.testOk = true;
    this.authorization = null;
  },

  async invoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    backend.calls.push({ cmd, args });
    const find = () => backend.sources.find((s) => s.source_id === args.sourceId)!;
    const lastTest = () =>
      backend.testOk ? { ok: true, code: null, message: "Connected. 12 order statuses available.", at: 1750000000 } : { ok: false, code: "AUTH_FAILED", message: "BaseLinker rejected the API token.", at: 1750000000 };
    switch (cmd) {
      case "get_state":
        return { app_version: "0.1.0", credential_store: { ok: true, message: null }, config_error: null, sources: structuredClone(backend.sources), providers } satisfies AppState;
      case "check_mcp": {
        const tools = backend.sources
          .filter((s) => s.enabled)
          .flatMap((s) => providers.find((p) => p.id === s.provider)!.tools.map((tool) => `${s.provider}__${s.source_id}__${tool.name}`));
        return { ok: true, server_version: "0.1.0", protocol_version: "2025-06-18", tools: ["ecommerce_mcp_list_sources", ...tools], error: null } satisfies McpCheck;
      }
      case "add_source": {
        // jak prawdziwy backend: pola niesekretne → settings, sekrety przyjęte i nigdy nieodesłane
        const provider = providers.find((p) => p.id === args.provider)!;
        const fields = args.fields as Record<string, string>;
        const settings = Object.fromEntries(provider.fields.filter((f) => !f.secret).map((f) => [f.key, fields[f.key] ?? ""]));
        const source: Source = {
          source_id: slug(args.name as string),
          provider: provider.id,
          name: args.name as string,
          enabled: true,
          created_at: 1750000000,
          settings,
          last_test: provider.auth === "oauth_device" ? NOT_CONNECTED : lastTest(),
        };
        backend.sources.push(source);
        return structuredClone(source);
      }
      case "update_source":
        if (args.name) find().name = args.name as string;
        return structuredClone(find());
      case "start_authorization":
        return { user_code: "abc-123-def", verification_uri: "https://allegro.pl/skojarz-aplikacje?code=abc123def", expires_in_secs: 3600 } satisfies AuthorizationView;
      case "finish_authorization":
        await new Promise<void>((resolve, reject) => {
          backend.authorization = { resolve, reject };
          if (backend.autoApproveMs !== null) setTimeout(resolve, backend.autoApproveMs);
        });
        find().last_test = { ok: true, code: null, message: "Connected as sklep_demo.", at: 1750000000 };
        return structuredClone(find());
      case "cancel_authorization":
        backend.authorization?.reject({ code: "AUTH_CANCELLED", message: "Authorization was cancelled." });
        return null;
      case "set_source_enabled":
        find().enabled = args.enabled as boolean;
        return find();
      case "test_source":
        find().last_test = lastTest();
        return find();
      case "delete_source":
        backend.sources = backend.sources.filter((s) => s.source_id !== args.sourceId);
        return null;
      case "client_setup": {
        const path = "/Applications/E-commerce MCP.app/Contents/MacOS/ecommerce-mcp";
        return {
          binary_path: path,
          claude_desktop_config_path: "~/Library/Application Support/Claude/claude_desktop_config.json",
          claude_desktop_json: JSON.stringify({ mcpServers: { "ecommerce-mcp": { command: path, args: ["mcp"] } } }, null, 2),
        } satisfies ClientSetup;
      }
      case "export_plugin":
        return "/Users/demo/Downloads/ecommerce-mcp-plugin.zip";
      case "diagnostic_report":
        return "E-commerce MCP 0.1.0\n(raport z podglądu)";
      case "open_data_dir":
        throw { code: "DATA_DIR_MISSING", message: "Data directory does not exist yet." };
      default:
        throw { code: "UNKNOWN", message: cmd };
    }
  },
};
