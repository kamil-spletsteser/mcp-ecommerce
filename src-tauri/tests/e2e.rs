//! E2E: prawdziwe binarium w trybie `mcp` (tak jak uruchomi je Claude/Codex) + mockowany BaseLinker po HTTP.
//! Pokrywa łańcuch: konfiguracja na dysku → credential store (fake z env, tylko debug) → MCP stdio → API.

use ecommerce_mcp::app::selfcheck::StdioSession;
use ecommerce_mcp::config::{self, Config, Source};
use serde_json::{json, Value};
use wiremock::matchers::{body_string_contains, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "4005-10023-E2ETOKENE2ETOKENE2ETOKEN0123456789ABCD";

fn source(id: &str, enabled: bool) -> Source {
    Source { source_id: id.into(), provider: "baselinker".into(), name: "Główny sklep".into(), enabled, created_at: 0, settings: json!({}), last_test: None }
}

fn save(dir: &std::path::Path, sources: Vec<Source>) {
    config::save(dir, &Config { schema_version: config::SCHEMA_VERSION, sources }).unwrap();
}

fn tool_names(listed: &Value) -> Vec<&str> {
    listed["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn ai_client_flow_over_stdio() {
    let baselinker = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("X-BLToken", TOKEN))
        .and(body_string_contains("method=getOrderStatusList"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "SUCCESS", "statuses": [{ "id": 1051, "name": "Nowe" }] })))
        .mount(&baselinker)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("method=getOrders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "SUCCESS", "orders": [{
            "order_id": 501, "order_status_id": 1051, "date_add": 1750000000, "date_confirmed": 1750000000, "currency": "PLN",
            "delivery_fullname": "Jan Kowalski", "delivery_price": 0, "products": [{ "name": "Kubek", "price_brutto": 20, "quantity": 1 }]
        }] })))
        .mount(&baselinker)
        .await;

    let data = tempfile::tempdir().unwrap();
    save(data.path(), vec![source("glowny_sklep", true)]);
    let secrets = json!({ "baselinker/glowny_sklep/api_token": TOKEN }).to_string();
    let envs = [
        ("ECOMMERCE_MCP_DATA_DIR", data.path().to_str().unwrap()),
        ("ECOMMERCE_MCP_TEST_SECRETS", secrets.as_str()),
        ("ECOMMERCE_MCP_BASELINKER_URL", &baselinker.uri()),
    ];
    let mut session = StdioSession::spawn(env!("CARGO_BIN_EXE_ecommerce-mcp").as_ref(), &envs).unwrap();

    // 1. handshake: serwer przedstawia się i deklaruje narzędzia
    let info = session.initialize().await.unwrap();
    assert_eq!(info["serverInfo"]["name"], "ecommerce-mcp");
    assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(info["capabilities"]["tools"].is_object());

    // 2. klient widzi narzędzia aktywnego źródła, ze schematami wejścia
    let listed = session.request("tools/list", json!({})).await.unwrap();
    let names = tool_names(&listed);
    assert!(names.contains(&"baselinker__glowny_sklep__get_order_statuses") && names.contains(&"baselinker__glowny_sklep__get_order"), "{names:?}");
    let get_order = listed["tools"].as_array().unwrap().iter().find(|t| t["name"] == "baselinker__glowny_sklep__get_order").unwrap();
    assert_eq!(get_order["inputSchema"]["required"], json!(["order_id"]));

    // 3. lista statusów i szczegóły zamówienia (kryterium akceptacji nr 7)
    let statuses = session.request("tools/call", json!({ "name": "baselinker__glowny_sklep__get_order_statuses", "arguments": {} })).await.unwrap();
    assert_eq!(statuses["structuredContent"]["statuses"][0]["name"], "Nowe");
    assert_ne!(statuses["isError"], true);
    let order = session.request("tools/call", json!({ "name": "baselinker__glowny_sklep__get_order", "arguments": { "order_id": 501 } })).await.unwrap();
    assert_eq!(order["structuredContent"]["order"]["delivery_fullname"], "Jan Kowalski");

    // 4. błędna walidacja wraca jako wynik narzędzia z kodem, nie jako awaria protokołu
    let invalid = session.request("tools/call", json!({ "name": "baselinker__glowny_sklep__get_order", "arguments": { "order_id": "abc" } })).await.unwrap();
    assert_eq!(invalid["isError"], true);
    assert_eq!(invalid["structuredContent"]["error"]["code"], "VALIDATION_ERROR");

    // 5. wyłączenie źródła w GUI (zapis konfiguracji): narzędzia znikają po odświeżeniu, wywołanie zwraca SOURCE_DISABLED
    save(data.path(), vec![source("glowny_sklep", false)]);
    let listed = session.request("tools/list", json!({})).await.unwrap();
    assert_eq!(tool_names(&listed), vec!["ecommerce_mcp_list_sources"]);
    let disabled = session.request("tools/call", json!({ "name": "baselinker__glowny_sklep__get_order_statuses", "arguments": {} })).await.unwrap();
    assert_eq!(disabled["structuredContent"]["error"]["code"], "SOURCE_DISABLED");

    // 6. token nie pojawił się w żadnej odpowiedzi protokołu
    for response in [&info, &listed, &statuses, &order, &invalid, &disabled] {
        assert!(!response.to_string().contains("E2ETOKEN"));
    }

    // 7. kontrolowane wyjście po zamknięciu stdin
    assert_eq!(session.shutdown().await, Some(0));
}

#[tokio::test]
async fn selfcheck_works_with_no_configuration_at_all() {
    let data = tempfile::tempdir().unwrap();
    let missing = data.path().join("never-created");
    let mut session = StdioSession::spawn(
        env!("CARGO_BIN_EXE_ecommerce-mcp").as_ref(),
        &[("ECOMMERCE_MCP_DATA_DIR", missing.to_str().unwrap()), ("ECOMMERCE_MCP_TEST_SECRETS", "{}")],
    )
    .unwrap();
    session.initialize().await.unwrap();
    let listed = session.request("tools/list", json!({})).await.unwrap();
    assert_eq!(tool_names(&listed), vec!["ecommerce_mcp_list_sources"]);
    session.shutdown().await;
    assert!(!missing.exists(), "serwer MCP nie tworzy katalogu danych");
}

#[tokio::test]
async fn allegro_tools_work_over_stdio_for_a_connected_account() {
    let allegro = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wiremock::matchers::path("/me"))
        .and(header("Authorization", "Bearer e2e-access-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "44", "login": "sklep_demo", "email": "sklep@example.com" })))
        .mount(&allegro)
        .await;

    let data = tempfile::tempdir().unwrap();
    let account = |id: &str| Source { provider: "allegro".into(), settings: json!({ "client_id": "0123456789abcdef0123456789abcdef" }), ..source(id, true) };
    save(data.path(), vec![account("polaczone"), account("niepolaczone")]);
    let secrets = json!({
        "allegro/polaczone/client_secret": "e2e-client-secret", "allegro/polaczone/access_token": "e2e-access-token", "allegro/polaczone/refresh_token": "e2e-refresh-token",
        "allegro/niepolaczone/client_secret": "e2e-client-secret"
    })
    .to_string();
    let envs = [
        ("ECOMMERCE_MCP_DATA_DIR", data.path().to_str().unwrap()),
        ("ECOMMERCE_MCP_TEST_SECRETS", secrets.as_str()),
        ("ECOMMERCE_MCP_ALLEGRO_URL", &allegro.uri()),
    ];
    let mut session = StdioSession::spawn(env!("CARGO_BIN_EXE_ecommerce-mcp").as_ref(), &envs).unwrap();
    session.initialize().await.unwrap();

    let listed = session.request("tools/list", json!({})).await.unwrap();
    assert!(tool_names(&listed).contains(&"allegro__polaczone__list_orders"));

    let account = session.request("tools/call", json!({ "name": "allegro__polaczone__get_account", "arguments": {} })).await.unwrap();
    assert_eq!(account["structuredContent"]["account"]["login"], "sklep_demo");

    // źródło dodane w GUI, ale bez dokończonej autoryzacji: czytelny błąd zamiast próby sieciowej
    let pending = session.request("tools/call", json!({ "name": "allegro__niepolaczone__get_account", "arguments": {} })).await.unwrap();
    assert_eq!(pending["structuredContent"]["error"]["code"], "CREDENTIAL_UNAVAILABLE");

    for response in [&listed, &account, &pending] {
        assert!(!response.to_string().contains("e2e-access-token") && !response.to_string().contains("e2e-client-secret"));
    }
    session.shutdown().await;
}
