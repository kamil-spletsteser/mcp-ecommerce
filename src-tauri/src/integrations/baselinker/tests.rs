//! Testy klienta i narzędzi BaseLinkera na mockowanym HTTP (wiremock) i fake'owym SecretStore.

use std::time::Duration;

use serde_json::{json, Value};
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::config::Source;
use crate::secrets::{MemoryStore, Secret, SecretKey, SecretStore};

const TOKEN: &str = "4005-10023-TESTTOKENTESTTOKENTESTTOKEN0123456789";

struct Fixture {
    provider: BaseLinker,
    source: Source,
    secrets: MemoryStore,
    server: MockServer,
}

impl Fixture {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let secrets = MemoryStore::default();
        secrets.set(&SecretKey::new("baselinker", "sklep", TOKEN_KIND), &Secret::new(TOKEN.into())).unwrap();
        Self {
            provider: BaseLinker::with_client(server.uri(), Duration::from_millis(300), Duration::from_millis(5)),
            source: Source {
                source_id: "sklep".into(),
                provider: "baselinker".into(),
                name: "Sklep".into(),
                enabled: true,
                created_at: 0,
                settings: json!({}),
                last_test: None,
            },
            secrets,
            server,
        }
    }

    async fn call(&self, tool: &str, args: Value) -> Result<Value, ToolError> {
        self.provider.call_tool(&SourceContext { source: &self.source, secrets: &self.secrets }, tool, &args).await
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }
}

/// Dopasowuje wywołanie metody API z dokładnie takimi parametrami (body: form `method` + `parameters` jako JSON).
fn api(expected_method: &'static str, expected_params: Value) -> impl Fn(&Request) -> bool + Send + Sync + 'static {
    move |request: &Request| {
        let form: std::collections::HashMap<String, String> = form_urlencoded::parse(&request.body).into_owned().collect();
        form.get("method").map(String::as_str) == Some(expected_method)
            && form.get("parameters").and_then(|p| serde_json::from_str::<Value>(p).ok()).as_ref() == Some(&expected_params)
    }
}

fn success(body: Value) -> ResponseTemplate {
    let mut body = body;
    body["status"] = "SUCCESS".into();
    ResponseTemplate::new(200).set_body_json(body)
}

fn order(id: i64, confirmed: i64) -> Value {
    json!({
        "order_id": id, "order_status_id": 1051, "date_add": confirmed - 60, "date_confirmed": confirmed, "order_source": "shop",
        "delivery_fullname": "Jan Kowalski", "email": "jan@example.com", "currency": "PLN", "payment_done": 0, "delivery_price": "10.00",
        "admin_comments": "", "invoice_company": "",
        "products": [{ "name": "Kubek", "sku": "K-1", "price_brutto": 20.5, "quantity": 2, "tax_rate": 23 }]
    })
}

#[tokio::test]
async fn success_sends_token_in_header_and_normalizes_output() {
    let f = Fixture::new().await;
    Mock::given(method("POST"))
        .and(header("X-BLToken", TOKEN))
        .and(api("getOrderStatusList", json!({})))
        .respond_with(success(
            json!({ "statuses": [{ "id": 1051, "name": "Nowe", "color": "#00f", "group_id": 1, "is_primary": 1, "name_for_customer": "" }] }),
        ))
        .expect(2) // narzędzie + test połączenia
        .mount(&f.server)
        .await;

    let out = f.call("get_order_statuses", json!({})).await.unwrap();
    assert_eq!(out, json!({ "statuses": [{ "id": 1051, "name": "Nowe", "color": "#00f" }] }));
    assert!(!out.to_string().contains(TOKEN));

    let ctx = SourceContext { source: &f.source, secrets: &f.secrets };
    assert_eq!(f.provider.test_connection(&ctx).await.unwrap(), "Connected. 1 order statuses available.");
}

#[tokio::test]
async fn bad_token_is_auth_failed_and_never_echoes_the_token() {
    let f = Fixture::new().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "status": "ERROR", "error_code": "ERROR_BAD_TOKEN", "error_message": format!("Bad token {TOKEN}") })),
        )
        .mount(&f.server)
        .await;
    let error = f.call("get_order_statuses", json!({})).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthFailed);
    assert!(!error.message.contains(TOKEN) && !error.message.contains("TESTTOKEN"), "{}", error.message);
    assert_eq!(f.requests().await, 1, "błąd autoryzacji nie jest ponawiany");
}

#[tokio::test]
async fn timeout_is_retried_for_reads_then_reported_as_upstream_error() {
    let f = Fixture::new().await;
    Mock::given(method("POST")).respond_with(success(json!({})).set_delay(Duration::from_secs(3))).mount(&f.server).await;
    let error = f.call("list_warehouses", json!({})).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(error.message.contains("timed out"), "{}", error.message);
    assert_eq!(f.requests().await, 3, "odczyt: 1 próba + 2 ponowienia");
}

#[tokio::test]
async fn rate_limit_is_mapped_and_not_retried() {
    let f = Fixture::new().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(429)).mount(&f.server).await;
    assert_eq!(f.call("list_inventories", json!({})).await.unwrap_err().code, ErrorCode::RateLimited);
    assert_eq!(f.requests().await, 1);

    assert_eq!(client::classify("ERROR_REQUESTS_LIMIT_EXCEEDED", "").code, ErrorCode::RateLimited);
    assert_eq!(client::classify("ERROR_USER_ACCOUNT_BLOCKED", "").code, ErrorCode::AuthFailed);
    assert_eq!(client::classify("ERROR_STORAGE_ID", "bad inventory").code, ErrorCode::UpstreamError);
}

#[tokio::test]
async fn upstream_failures_are_upstream_errors() {
    let f = Fixture::new().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.call("list_inventories", json!({})).await.unwrap_err().code, ErrorCode::UpstreamError);

    Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).mount(&f.server).await;
    assert_eq!(f.call("list_inventories", json!({})).await.unwrap_err().code, ErrorCode::UpstreamError);
    assert_eq!(f.requests().await, 1 + 3, "5xx przy odczycie jest ponawiane");
}

#[tokio::test]
async fn transient_error_recovers_on_retry() {
    let f = Fixture::new().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(502)).up_to_n_times(1).mount(&f.server).await;
    Mock::given(method("POST"))
        .respond_with(success(json!({ "warehouses": [{ "warehouse_id": 205, "warehouse_type": "bl", "name": "Główny", "address": "ul. Tajna 1" }] })))
        .mount(&f.server)
        .await;
    let out = f.call("list_warehouses", json!({})).await.unwrap();
    assert_eq!(out, json!({ "warehouses": [{ "warehouse_type": "bl", "warehouse_id": 205, "name": "Główny" }] }));
}

#[tokio::test]
async fn update_order_status_confirms_action_and_is_never_retried() {
    let f = Fixture::new().await;
    Mock::given(api("setOrderStatus", json!({ "order_id": 77, "status_id": 1052 }))).respond_with(success(json!({}))).expect(1).mount(&f.server).await;
    let out = f.call("update_order_status", json!({ "order_id": 77, "status_id": 1052 })).await.unwrap();
    assert_eq!(out, json!({ "ok": true, "action": "order_status_updated", "order_id": 77, "status_id": 1052 }));

    let failing = Fixture::new().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(500)).mount(&failing.server).await;
    assert_eq!(failing.call("update_order_status", json!({ "order_id": 77, "status_id": 1052 })).await.unwrap_err().code, ErrorCode::UpstreamError);
    assert_eq!(failing.requests().await, 1, "zapis nie może być ponawiany automatycznie");
}

#[tokio::test]
async fn add_order_note_appends_and_respects_the_200_char_limit() {
    let f = Fixture::new().await;
    let mut existing = order(77, 1_750_000_000);
    existing["admin_comments"] = "VIP".into();
    Mock::given(api("getOrders", json!({ "order_id": 77, "get_unconfirmed_orders": true })))
        .respond_with(success(json!({ "orders": [existing] })))
        .mount(&f.server)
        .await;
    Mock::given(api("setOrderFields", json!({ "order_id": 77, "admin_comments": "VIP | zadzwonić przed dostawą" })))
        .respond_with(success(json!({})))
        .expect(1)
        .mount(&f.server)
        .await;

    let out = f.call("add_order_note", json!({ "order_id": 77, "note": "zadzwonić przed dostawą" })).await.unwrap();
    assert_eq!(out["action"], "order_note_added");
    assert_eq!(out["admin_comments"], "VIP | zadzwonić przed dostawą");

    let too_long = f.call("add_order_note", json!({ "order_id": 77, "note": "x".repeat(198) })).await.unwrap_err();
    assert_eq!(too_long.code, ErrorCode::ValidationError);
    // .expect(1) na setOrderFields pilnuje, że za długa notatka nie wywołała zapisu
}

#[tokio::test]
async fn list_orders_maps_filters_and_paginates() {
    let f = Fixture::new().await;
    let base = 1_750_000_000; // 2025-06-15T15:06:40Z
    let orders: Vec<Value> = (0..3).map(|i| order(100 + i, base + i * 10)).collect();
    Mock::given(api("getOrders", json!({ "date_confirmed_from": 1_749_945_600, "status_id": 1051, "filter_email": "jan@example.com" })))
        .respond_with(success(json!({ "orders": orders })))
        .mount(&f.server)
        .await;

    let out = f.call("list_orders", json!({ "date_from": "2025-06-15", "status_id": 1051, "email": "jan@example.com", "limit": 2 })).await.unwrap();
    assert_eq!(out["returned"], 2);
    assert_eq!(out["has_more"], true);
    assert_eq!(out["next_date_from"], "2025-06-15T15:06:51Z");
    let first = &out["orders"][0];
    assert_eq!(first["order_id"], 100);
    assert_eq!(first["date_confirmed"], "2025-06-15T15:06:40Z");
    assert_eq!(first["total_gross"], 51.0);
    assert_eq!(first["products_count"], 1);
    assert!(first.get("invoice_company").is_none() && first.get("products").is_none(), "podsumowanie nie zawiera pełnych danych");

    // date_to filtruje po stronie klienta i kończy paginację
    let out = f
        .call("list_orders", json!({ "date_from": "2025-06-15", "date_to": "2025-06-15T15:06:45Z", "status_id": 1051, "email": "jan@example.com" }))
        .await
        .unwrap();
    assert_eq!((out["returned"].clone(), out["has_more"].clone(), out["next_date_from"].clone()), (json!(1), json!(false), Value::Null));
}

#[tokio::test]
async fn list_orders_validates_input_before_any_request() {
    let f = Fixture::new().await;
    for args in [
        json!({ "date_from": "15.06.2025" }),
        json!({ "date_from": "1999-12-31" }),
        json!({ "date_from": "2999-01-01" }),
        json!({ "date_from": "2025-06-15", "date_to": "2025-06-01" }),
        json!({ "date_from": "2024-01-01", "date_to": "2025-06-01" }),
        json!({ "limit": 101 }),
        json!({ "limit": 0 }),
        json!({ "status_id": -1 }),
        json!({ "url": "https://evil.example" }),
    ] {
        assert_eq!(f.call("list_orders", args.clone()).await.unwrap_err().code, ErrorCode::ValidationError, "{args}");
    }
    assert_eq!(f.requests().await, 0);
}

#[tokio::test]
async fn get_order_returns_details_or_not_found() {
    let f = Fixture::new().await;
    Mock::given(api("getOrders", json!({ "order_id": 100, "get_unconfirmed_orders": true })))
        .respond_with(success(json!({ "orders": [order(100, 1_750_000_000)] })))
        .mount(&f.server)
        .await;
    Mock::given(api("getOrders", json!({ "order_id": 404, "get_unconfirmed_orders": true })))
        .respond_with(success(json!({ "orders": [] })))
        .mount(&f.server)
        .await;

    let out = f.call("get_order", json!({ "order_id": 100 })).await.unwrap();
    assert_eq!(out["order"]["products"][0], json!({ "name": "Kubek", "sku": "K-1", "price_brutto": 20.5, "tax_rate": 23, "quantity": 2 }));
    assert_eq!(out["order"]["delivery_fullname"], "Jan Kowalski");
    assert!(out["order"].get("invoice_company").is_none(), "puste pola są pomijane");

    assert_eq!(f.call("get_order", json!({ "order_id": 404 })).await.unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn list_products_flattens_map_and_flags_truncation() {
    let f = Fixture::new().await;
    Mock::given(api("getInventoryProductsList", json!({ "inventory_id": 306, "page": 1, "filter_name": "kubek" })))
        .respond_with(success(json!({ "products": {
            "20": { "id": 20, "sku": "K-2", "ean": "", "name": "Kubek B", "prices": { "105": 25.0 }, "stock": { "bl_205": 3 } },
            "10": { "id": 10, "sku": "K-1", "ean": "590", "name": "Kubek A", "prices": { "105": 20.5 }, "stock": { "bl_205": 0 } }
        } })))
        .mount(&f.server)
        .await;
    Mock::given(api("getInventoryProductsList", json!({ "inventory_id": 307, "page": 2 })))
        .respond_with(success(json!({ "products": [] })))
        .mount(&f.server)
        .await;

    let out = f.call("list_products", json!({ "inventory_id": 306, "name": "kubek", "limit": 1 })).await.unwrap();
    assert_eq!(out["products"], json!([{ "id": 10, "sku": "K-1", "ean": "590", "name": "Kubek A", "prices": { "105": 20.5 }, "stock": { "bl_205": 0 } }]));
    assert_eq!((out["truncated"].clone(), out["has_next_page"].clone()), (json!(true), json!(false)));

    let empty = f.call("list_products", json!({ "inventory_id": 307, "page": 2 })).await.unwrap();
    assert_eq!(empty["returned"], 0);
    assert_eq!(f.call("list_products", json!({})).await.unwrap_err().code, ErrorCode::ValidationError);
}

#[tokio::test]
async fn missing_credential_is_reported_without_network() {
    let f = Fixture::new().await;
    f.secrets.delete(&SecretKey::new("baselinker", "sklep", TOKEN_KIND)).unwrap();
    assert_eq!(f.call("get_order_statuses", json!({})).await.unwrap_err().code, ErrorCode::CredentialUnavailable);
    assert_eq!(f.requests().await, 0);
}

#[test]
fn token_validation_is_local() {
    let provider = BaseLinker::default();
    assert!(provider.validate_field(TOKEN_KIND, TOKEN).is_ok());
    for bad in ["", "short", "has space inside the token value", &"x".repeat(301)] {
        assert_eq!(provider.validate_field(TOKEN_KIND, bad).unwrap_err().code, ErrorCode::ValidationError);
    }
}
