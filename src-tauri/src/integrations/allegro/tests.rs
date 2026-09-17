//! Testy providera Allegro na mockowanym HTTP (wiremock) i fake'owym SecretStore. Jeden serwer udaje oba hosty (API i logowanie).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::config::Source;
use crate::secrets::{MemoryStore, SecretKey, SecretStore};

const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";
const CLIENT_SECRET: &str = "ClientSecretClientSecretClientSecret0123456789";
/// base64("0123456789abcdef0123456789abcdef:ClientSecretClientSecretClientSecret0123456789")
const BASIC: &str = "Basic MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY6Q2xpZW50U2VjcmV0Q2xpZW50U2VjcmV0Q2xpZW50U2VjcmV0MDEyMzQ1Njc4OQ==";
const ORDER_ID: &str = "29738e61-7f6a-11e8-ac45-09db60ede9d6";

struct Fixture {
    provider: Allegro,
    source: Source,
    secrets: Arc<MemoryStore>,
    server: MockServer,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let secrets = Arc::new(MemoryStore::default());
        let dir = tempfile::tempdir().unwrap();
        let fixture = Self {
            provider: Allegro::with_urls(server.uri(), server.uri(), Duration::from_millis(300), Duration::from_millis(5), dir.path().join("refresh.lock")),
            source: Source {
                source_id: "moje_allegro".into(),
                provider: "allegro".into(),
                name: "Moje Allegro".into(),
                enabled: true,
                created_at: 0,
                settings: json!({ "client_id": CLIENT_ID }),
                last_test: None,
            },
            secrets,
            server,
            _dir: dir,
        };
        fixture.store("client_secret", CLIENT_SECRET);
        fixture
    }

    /// Jak po udanej autoryzacji: para tokenów w credential store.
    async fn connected() -> Self {
        let fixture = Self::new().await;
        fixture.store("access_token", "access-token-AAA");
        fixture.store("refresh_token", "refresh-token-AAA");
        fixture
    }

    fn store(&self, kind: &str, value: &str) {
        self.secrets.set(&SecretKey::new("allegro", "moje_allegro", kind), &Secret::new(value.into())).unwrap();
    }

    fn stored(&self, kind: &str) -> Option<String> {
        self.secrets.get(&SecretKey::new("allegro", "moje_allegro", kind)).unwrap().map(|s| s.expose().to_string())
    }

    fn ctx(&self) -> SourceContext<'_> {
        SourceContext { source: &self.source, secrets: self.secrets.as_ref() }
    }

    async fn call(&self, tool: &str, args: Value) -> Result<Value, ToolError> {
        self.provider.call_tool(&self.ctx(), tool, &args).await
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }
}

fn tokens(suffix: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "access_token": format!("access-token-{suffix}"), "refresh_token": format!("refresh-token-{suffix}"), "expires_in": 43199, "token_type": "bearer" }))
}

fn oauth_error(error: &str) -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({ "error": error, "error_description": "…" }))
}

#[tokio::test]
async fn device_flow_starts_with_app_credentials_and_read_only_scopes() {
    let f = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/auth/oauth/device"))
        .and(header("Authorization", BASIC))
        .and(body_string_contains(format!("client_id={CLIENT_ID}")))
        .and(body_string_contains("allegro%3Aapi%3Aorders%3Aread"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "device-code-XYZ", "user_code": "abc-123-def", "interval": 5, "expires_in": 3600,
            "verification_uri": format!("{}/skojarz-aplikacje", f.server.uri()),
            "verification_uri_complete": format!("{}/skojarz-aplikacje?code=abc123def", f.server.uri()),
        })))
        .mount(&f.server)
        .await;

    let auth = f.provider.begin_authorization(&f.ctx()).await.unwrap();
    assert_eq!((auth.user_code.as_str(), auth.interval_secs, auth.expires_in_secs), ("abc-123-def", 5, 3600));
    assert!(auth.verification_uri.ends_with("/skojarz-aplikacje?code=abc123def"));
    assert_eq!(auth.device_code.expose(), "device-code-XYZ");

    let body = String::from_utf8(f.server.received_requests().await.unwrap()[0].body.clone()).unwrap();
    assert!(!body.contains("write"), "wersja tylko do odczytu nie prosi o uprawnienia zapisu: {body}");
}

#[tokio::test]
async fn device_flow_rejects_bad_credentials_and_foreign_verification_address() {
    let f = Fixture::new().await;
    Mock::given(path("/auth/oauth/device"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "invalid_client" })))
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    let error = f.provider.begin_authorization(&f.ctx()).await.err().unwrap();
    assert_eq!(error.code, ErrorCode::AuthFailed);
    assert!(!error.message.contains(CLIENT_SECRET));

    // adres spoza serwisu logowania Allegro nie może trafić do przeglądarki użytkownika
    Mock::given(path("/auth/oauth/device"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "device_code": "d", "user_code": "u", "verification_uri_complete": "https://evil.example/login" })),
        )
        .mount(&f.server)
        .await;
    assert_eq!(f.provider.begin_authorization(&f.ctx()).await.err().unwrap().code, ErrorCode::UpstreamError);
}

#[tokio::test]
async fn polling_maps_oauth_states_and_stores_tokens_on_success() {
    let f = Fixture::new().await;
    let device_code = Secret::new("device-code-XYZ".into());
    let token_endpoint = || {
        Mock::given(method("POST")).and(path("/auth/oauth/token")).and(header("Authorization", BASIC)).and(body_string_contains("device_code=device-code-XYZ"))
    };

    token_endpoint().respond_with(oauth_error("authorization_pending")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap(), AuthPoll::Pending);
    assert_eq!(f.stored("access_token"), None);

    token_endpoint().respond_with(oauth_error("slow_down")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap(), AuthPoll::SlowDown);

    token_endpoint().respond_with(tokens("NEW")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap(), AuthPoll::Done);
    assert_eq!((f.stored("access_token").as_deref(), f.stored("refresh_token").as_deref()), (Some("access-token-NEW"), Some("refresh-token-NEW")));

    token_endpoint().respond_with(ResponseTemplate::new(429).set_body_json(json!({}))).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap(), AuthPoll::SlowDown, "429 = odpytuj rzadziej, nie koniec autoryzacji");

    token_endpoint().respond_with(oauth_error("access_denied")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap_err().code, ErrorCode::AuthFailed);
    token_endpoint().respond_with(oauth_error("invalid_grant")).mount(&f.server).await;
    assert_eq!(f.provider.poll_authorization(&f.ctx(), &device_code).await.unwrap_err().code, ErrorCode::AuthFailed);
}

#[tokio::test]
async fn expired_access_token_is_refreshed_once_and_rotation_is_persisted() {
    let f = Fixture::connected().await;
    Mock::given(path("/me")).and(header("Authorization", "Bearer access-token-AAA")).respond_with(ResponseTemplate::new(401)).mount(&f.server).await;
    Mock::given(path("/auth/oauth/token"))
        .and(header("Authorization", BASIC))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=refresh-token-AAA"))
        .respond_with(tokens("BBB"))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(path("/me"))
        .and(header("Authorization", "Bearer access-token-BBB"))
        .and(header("Accept", "application/vnd.allegro.public.v1+json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "44", "login": "sklep_demo", "email": "a@b.pl", "firstName": "Jan" })))
        .mount(&f.server)
        .await;

    assert_eq!(f.provider.test_connection(&f.ctx()).await.unwrap(), "Connected as sklep_demo.");
    // refresh token jest jednorazowy — nowa para musi wylądować w credential store
    assert_eq!((f.stored("access_token").as_deref(), f.stored("refresh_token").as_deref()), (Some("access-token-BBB"), Some("refresh-token-BBB")));

    let out = f.call("get_account", json!({})).await.unwrap();
    assert_eq!(out, json!({ "account": { "id": "44", "login": "sklep_demo", "email": "a@b.pl" } }));
    for secret in ["access-token", "refresh-token", CLIENT_SECRET] {
        assert!(!out.to_string().contains(secret));
    }
}

#[tokio::test]
async fn concurrent_calls_with_a_stale_token_refresh_exactly_once() {
    let f = Fixture::connected().await;
    Mock::given(path("/me")).and(header("Authorization", "Bearer access-token-AAA")).respond_with(ResponseTemplate::new(401)).mount(&f.server).await;
    Mock::given(path("/me"))
        .and(header("Authorization", "Bearer access-token-BBB"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "login": "sklep_demo" })))
        .mount(&f.server)
        .await;
    // drugi refresh tym samym (jednorazowym) tokenem zostałby odrzucony — .expect(1) pilnuje, że blokada go nie dopuściła
    Mock::given(path("/auth/oauth/token")).respond_with(tokens("BBB").set_delay(Duration::from_millis(50))).expect(1).mount(&f.server).await;

    let ctx = f.ctx();
    let (a, b, c) = tokio::join!(f.provider.test_connection(&ctx), f.provider.test_connection(&ctx), f.provider.test_connection(&ctx));
    assert!(a.is_ok() && b.is_ok() && c.is_ok(), "{a:?} {b:?} {c:?}");
}

#[tokio::test]
async fn rejected_refresh_means_reconnect_unless_another_process_already_rotated() {
    // 1) refresh token naprawdę martwy → AUTH_FAILED z prośbą o ponowne połączenie
    let f = Fixture::connected().await;
    Mock::given(path("/me")).respond_with(ResponseTemplate::new(401)).mount(&f.server).await;
    Mock::given(path("/auth/oauth/token")).respond_with(oauth_error("invalid_grant")).mount(&f.server).await;
    let error = f.provider.test_connection(&f.ctx()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthFailed);
    assert!(error.message.contains("Reconnect"), "{}", error.message);

    // 2) inny proces (GUI / druga sesja MCP) zrotował tokeny w trakcie naszego odświeżania → bierzemy jego parę
    let f = Fixture::connected().await;
    Mock::given(path("/me")).and(header("Authorization", "Bearer access-token-AAA")).respond_with(ResponseTemplate::new(401)).mount(&f.server).await;
    Mock::given(path("/me"))
        .and(header("Authorization", "Bearer access-token-OTHER"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "login": "sklep_demo" })))
        .mount(&f.server)
        .await;
    let store = f.secrets.clone();
    Mock::given(path("/auth/oauth/token"))
        .respond_with(move |_: &Request| {
            for (kind, value) in [("refresh_token", "refresh-token-OTHER"), ("access_token", "access-token-OTHER")] {
                store.set(&SecretKey::new("allegro", "moje_allegro", kind), &Secret::new(value.into())).unwrap();
            }
            oauth_error("invalid_grant")
        })
        .mount(&f.server)
        .await;
    assert_eq!(f.provider.test_connection(&f.ctx()).await.unwrap(), "Connected as sklep_demo.");
}

#[tokio::test]
async fn not_connected_account_is_reported_without_network() {
    let f = Fixture::new().await;
    let error = f.call("list_orders", json!({})).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::CredentialUnavailable);
    assert!(error.message.contains("not connected"));
    assert_eq!(f.requests().await, 0);
}

#[tokio::test]
async fn list_orders_maps_filters_and_pages() {
    let f = Fixture::connected().await;
    let form = |id: &str| {
        json!({
            "id": id, "status": "READY_FOR_PROCESSING", "updatedAt": "2025-06-15T16:00:00Z", "revision": "r1",
            "fulfillment": { "status": "NEW" }, "buyer": { "login": "kupujacy1", "email": "k@allegromail.pl", "address": { "street": "Tajna 1" } },
            "payment": { "type": "ONLINE", "finishedAt": "2025-06-15T15:10:00Z" }, "delivery": { "method": { "name": "Allegro Paczkomaty InPost" } },
            "summary": { "totalToPay": { "amount": "59.99", "currency": "PLN" } },
            "lineItems": [{ "id": "li1", "offer": { "id": "123", "name": "Kubek" }, "quantity": 2, "price": { "amount": "25.00", "currency": "PLN" }, "boughtAt": "2025-06-15T15:06:40Z" }]
        })
    };
    Mock::given(method("GET"))
        .and(path("/order/checkout-forms"))
        .and(query_param("status", "READY_FOR_PROCESSING"))
        .and(query_param("fulfillment.status", "NEW"))
        .and(query_param("lineItems.boughtAt.gte", "2025-06-15T00:00:00Z"))
        .and(query_param("lineItems.boughtAt.lte", "2025-06-15T23:59:59Z"))
        .and(query_param("buyer.login", "kupujacy1"))
        .and(query_param("limit", "2"))
        .and(query_param("offset", "0"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "checkoutForms": [form(ORDER_ID), form("39738e61-7f6a-11e8-ac45-09db60ede9d6")], "count": 2, "totalCount": 5 })),
        )
        .mount(&f.server)
        .await;

    let args = json!({ "status": "READY_FOR_PROCESSING", "fulfillment_status": "NEW", "bought_from": "2025-06-15", "bought_to": "2025-06-15", "buyer_login": "kupujacy1", "limit": 2 });
    let out = f.call("list_orders", args).await.unwrap();
    assert_eq!((out["returned"].clone(), out["total_count"].clone(), out["next_offset"].clone()), (json!(2), json!(5), json!(2)));
    assert_eq!(
        out["orders"][0],
        json!({
            "id": ORDER_ID, "status": "READY_FOR_PROCESSING", "updatedAt": "2025-06-15T16:00:00Z", "fulfillment_status": "NEW", "buyer_login": "kupujacy1",
            "bought_at": "2025-06-15T15:06:40Z", "total_to_pay": { "amount": "59.99", "currency": "PLN" }, "payment_type": "ONLINE",
            "paid_at": "2025-06-15T15:10:00Z", "delivery_method": "Allegro Paczkomaty InPost", "items_count": 1
        }),
        "podsumowanie bez adresu i e-maila kupującego"
    );
}

#[tokio::test]
async fn input_is_validated_before_any_request() {
    let f = Fixture::connected().await;
    for (tool, args) in [
        ("list_orders", json!({ "status": "SENT" })),
        ("list_orders", json!({ "fulfillment_status": "DONE" })),
        ("list_orders", json!({ "bought_from": "15.06.2025" })),
        ("list_orders", json!({ "bought_from": "2025-06-15", "bought_to": "2025-06-01" })),
        ("list_orders", json!({ "limit": 101 })),
        ("list_orders", json!({ "limit": 100, "offset": 9950 })),
        ("list_orders", json!({ "url": "https://evil.example" })),
        ("get_order", json!({ "order_id": "../../sale/offers" })),
        ("get_order", json!({})),
        ("get_offer", json!({ "offer_id": "12a/../me" })),
        ("list_offers", json!({ "status": "DELETED" })),
        ("list_offers", json!({ "limit": 201 })),
        ("get_account", json!({ "x": 1 })),
        ("delete_offer", json!({})),
    ] {
        assert_eq!(f.call(tool, args.clone()).await.unwrap_err().code, ErrorCode::ValidationError, "{tool} {args}");
    }
    assert_eq!(f.requests().await, 0);
}

#[tokio::test]
async fn order_and_offer_details_are_whitelisted() {
    let f = Fixture::connected().await;
    Mock::given(path(format!("/order/checkout-forms/{ORDER_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ORDER_ID, "status": "READY_FOR_PROCESSING", "messageToSeller": "Proszę o fakturę", "buyer": { "login": "kupujacy1" }, "internalNote": "x",
            "lineItems": [{ "id": "li1", "offer": { "id": "123", "name": "Kubek" }, "quantity": 1, "price": { "amount": "25.00", "currency": "PLN" }, "reconciliation": { "x": 1 } }]
        })))
        .mount(&f.server)
        .await;
    Mock::given(path("/sale/product-offers/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123", "name": "Kubek", "stock": { "available": 7 }, "description": { "sections": [{ "items": [{ "type": "TEXT", "content": "<p>długi opis</p>" }] }] },
            "images": ["https://a.allegroimg.com/1", "https://a.allegroimg.com/2"], "productSet": [{ "product": { "id": "p-1", "name": "Kubek ceramiczny", "parameters": [] }, "quantity": { "value": 1 } }]
        })))
        .mount(&f.server)
        .await;
    Mock::given(path("/order/checkout-forms/00000000-0000-0000-0000-000000000000"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(
                json!({ "errors": [{ "code": "NOT_FOUND", "message": "Checkout form not found", "userMessage": "Nie znaleziono zamówienia." }] }),
            ),
        )
        .mount(&f.server)
        .await;

    let order = f.call("get_order", json!({ "order_id": ORDER_ID })).await.unwrap();
    assert_eq!(order["order"]["messageToSeller"], "Proszę o fakturę");
    assert_eq!(
        order["order"]["lineItems"][0],
        json!({ "id": "li1", "offer": { "id": "123", "name": "Kubek" }, "quantity": 1, "price": { "amount": "25.00", "currency": "PLN" } })
    );
    assert!(order["order"].get("internalNote").is_none());

    let offer = f.call("get_offer", json!({ "offer_id": "123" })).await.unwrap();
    assert_eq!(offer["offer"]["images_count"], 2);
    assert_eq!(offer["offer"]["products"], json!([{ "product": { "id": "p-1", "name": "Kubek ceramiczny" }, "quantity": 1 }]));
    assert!(offer["offer"].get("description").is_none(), "opis HTML nie trafia do modelu");

    let missing = f.call("get_order", json!({ "order_id": "00000000-0000-0000-0000-000000000000" })).await.unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    assert!(missing.message.contains("Nie znaleziono zamówienia."));
}

#[tokio::test]
async fn list_offers_maps_filters() {
    let f = Fixture::connected().await;
    Mock::given(path("/sale/offers"))
        .and(query_param("name", "kubek"))
        .and(query_param("publication.status", "ACTIVE"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "offers": [{ "id": "123", "name": "Kubek", "sellingMode": { "price": { "amount": "25.00", "currency": "PLN" } }, "stock": { "available": 7, "sold": 3 },
                         "publication": { "status": "ACTIVE" }, "primaryImage": { "url": "https://a.allegroimg.com/1" } }],
            "count": 1, "totalCount": 1
        })))
        .mount(&f.server)
        .await;
    let out = f.call("list_offers", json!({ "name": "kubek", "status": "ACTIVE" })).await.unwrap();
    assert_eq!(
        out["offers"][0],
        json!({ "id": "123", "name": "Kubek", "sellingMode": { "price": { "amount": "25.00", "currency": "PLN" } }, "stock": { "available": 7, "sold": 3 }, "publication": { "status": "ACTIVE" } })
    );
    assert_eq!(out["next_offset"], Value::Null);
}

#[tokio::test]
async fn upstream_statuses_are_normalized() {
    let f = Fixture::connected().await;
    Mock::given(path("/me")).respond_with(ResponseTemplate::new(403)).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.call("get_account", json!({})).await.unwrap_err().code, ErrorCode::AuthFailed);

    Mock::given(path("/me")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30")).up_to_n_times(1).mount(&f.server).await;
    assert_eq!(f.call("get_account", json!({})).await.unwrap_err().code, ErrorCode::RateLimited);
    assert_eq!(f.requests().await, 2, "ani 403, ani 429 nie są ponawiane");

    Mock::given(path("/me")).respond_with(ResponseTemplate::new(503)).mount(&f.server).await;
    assert_eq!(f.call("get_account", json!({})).await.unwrap_err().code, ErrorCode::UpstreamError);
    assert_eq!(f.requests().await, 2 + 3, "5xx przy odczycie: 1 próba + 2 ponowienia");
}

#[test]
fn form_fields_are_validated_locally() {
    let provider = Allegro::default();
    assert!(provider.validate_field("client_id", CLIENT_ID).is_ok());
    assert!(provider.validate_field("client_secret", CLIENT_SECRET).is_ok());
    for (key, bad) in [
        ("client_id", "short"),
        ("client_id", "has space in the client id"),
        ("client_secret", ""),
        ("client_secret", "sekret ze spacją w środku"),
        ("token", CLIENT_SECRET),
    ] {
        assert_eq!(provider.validate_field(key, bad).unwrap_err().code, ErrorCode::ValidationError, "{key}={bad}");
    }
}
