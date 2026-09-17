//! Allegro — konto sprzedawcy przez Allegro REST API (stan dokumentacji: 2026-09). Wersja 1: tylko odczyt.
//!
//! Każdy użytkownik rejestruje własną aplikację typu „device” w apps.developer.allegro.pl i podaje jej Client ID
//! (niesekretny → `Source.settings`) oraz Client Secret (credential store). Konto łączy się przez OAuth Device Flow (`auth.rs`).
//! Użyte endpointy: `GET /me`, `GET /order/checkout-forms[/{id}]`, `GET /sale/offers`, `GET /sale/product-offers/{id}`.

mod auth;
#[cfg(test)]
mod tests;

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::http::{self, Attempt};
use super::{
    args_object, iso, object_schema as schema, opt_date, opt_enum, opt_int, opt_str, pick, pick_each, AuthKind, AuthPoll, Capability, DeviceAuthorization,
    ErrorCode, FieldSpec, Provider, ProviderMeta, SourceContext, ToolDef, ToolError,
};
use crate::secrets::Secret;

const SERVICE: &str = "Allegro";
const ORDER_STATUSES: [&str; 4] = ["BOUGHT", "FILLED_IN", "READY_FOR_PROCESSING", "CANCELLED"];
const FULFILLMENT_STATUSES: [&str; 9] =
    ["NEW", "PROCESSING", "READY_FOR_SHIPMENT", "READY_FOR_PICKUP", "SENT", "PICKED_UP", "CANCELLED", "SUSPENDED", "RETURNED"];
const OFFER_STATUSES: [&str; 4] = ["INACTIVE", "ACTIVE", "ACTIVATING", "ENDED"];
/// Allegro: `limit + offset` dla zamówień nie może przekroczyć 10 000.
const ORDERS_WINDOW: i64 = 10_000;

pub struct Allegro {
    http: reqwest::Client,
    api_base: String,
    auth_base: String,
    backoff: Duration,
    /// Plik-blokada odświeżania tokenów (patrz `auth.rs`).
    lock_path: std::path::PathBuf,
}

impl Default for Allegro {
    fn default() -> Self {
        let lock_path = crate::config::data_dir().join("allegro-refresh.lock");
        // Hosty są stałe — żadne narzędzie ani ustawienie nie pozwala podać własnego adresu.
        // Nadpisanie wyłącznie w buildach debug na potrzeby testów E2E z mockiem (jeden serwer udaje API i logowanie).
        #[cfg(debug_assertions)]
        if let Ok(url) = std::env::var("ECOMMERCE_MCP_ALLEGRO_URL") {
            return Self::with_urls(url.clone(), url, Duration::from_secs(30), Duration::from_millis(500), lock_path);
        }
        Self::with_urls("https://api.allegro.pl".into(), "https://allegro.pl".into(), Duration::from_secs(30), Duration::from_millis(500), lock_path)
    }
}

enum Fetched {
    Json(Value),
    /// 401: access token wygasł albo został unieważniony.
    Unauthorized,
}

impl Allegro {
    pub fn with_urls(api_base: String, auth_base: String, timeout: Duration, backoff: Duration, lock_path: std::path::PathBuf) -> Self {
        Self { http: http::client(timeout), api_base, auth_base, backoff, lock_path }
    }

    /// GET z automatycznym, jednorazowym odświeżeniem tokenu po 401. Tylko odczyty → ponowienia dla błędów przejściowych.
    async fn get(&self, ctx: &SourceContext<'_>, path: &str, query: &[(&str, String)]) -> Result<Value, ToolError> {
        let mut access = ctx.secret(auth::ACCESS_TOKEN).map_err(auth::not_connected)?;
        for refreshed in [false, true] {
            match http::with_retries(&format!("allegro GET {path}"), true, self.backoff, || self.get_once(&access, path, query)).await? {
                Fetched::Json(json) => return Ok(json),
                Fetched::Unauthorized if !refreshed => access = self.refresh(ctx, &access).await?,
                Fetched::Unauthorized => break,
            }
        }
        Err(auth::unauthorized_after_refresh())
    }

    async fn get_once(&self, access: &Secret, path: &str, query: &[(&str, String)]) -> Attempt<Fetched> {
        let response = self
            .http
            .get(format!("{}{path}", self.api_base))
            .bearer_auth(access.expose())
            .header("Accept", "application/vnd.allegro.public.v1+json")
            .header("Accept-Language", "pl-PL")
            .query(query)
            .send()
            .await
            .map_err(|e| http::send_error(&e, SERVICE))?;

        let status = response.status().as_u16();
        match status {
            401 => return Ok(Fetched::Unauthorized),
            500..=599 => return Err(http::upstream(format!("Allegro is unavailable (HTTP {status})."), true)),
            _ => {}
        }
        let body = http::read_capped(response, SERVICE).await?;
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return if json.is_object() { Ok(Fetched::Json(json)) } else { Err(http::upstream("Allegro returned a non-JSON response.", false)) };
        }
        // {"errors":[{"code","message","userMessage"}]}
        let detail = json["errors"][0]["userMessage"].as_str().or(json["errors"][0]["message"].as_str()).unwrap_or("");
        let error = match status {
            403 => ToolError::new(
                ErrorCode::AuthFailed,
                "Allegro denied access: the connected app lacks the required permission. Enable reading orders, offers and profile for the app at apps.developer.allegro.pl, then reconnect.",
            ),
            404 => ToolError::new(ErrorCode::NotFound, format!("Not found in Allegro. {detail}")),
            400 | 422 => ToolError::validation(format!("Allegro rejected the request: {detail}")),
            429 => ToolError::new(ErrorCode::RateLimited, "Allegro API rate limit reached. Wait a minute and try again."),
            _ => ToolError::new(ErrorCode::UpstreamError, format!("Unexpected Allegro response (HTTP {status}). {detail}")),
        };
        Err((error, false))
    }
}

#[async_trait]
impl Provider for Allegro {
    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "allegro",
            name: "Allegro",
            auth: AuthKind::OauthDevice,
            fields: vec![
                FieldSpec { key: auth::CLIENT_ID, secret: false, required: true, max_len: 100 },
                FieldSpec { key: auth::CLIENT_SECRET, secret: true, required: true, max_len: 200 },
            ],
            capabilities: vec![
                Capability { label_key: "cap.orders.read", write: false, tools: vec!["list_orders", "get_order"] },
                Capability { label_key: "cap.offers.read", write: false, tools: vec!["list_offers", "get_offer"] },
                Capability { label_key: "cap.account.read", write: false, tools: vec!["get_account"] },
            ],
        }
    }

    fn validate_field(&self, key: &str, value: &str) -> Result<(), ToolError> {
        let ok = match key {
            auth::CLIENT_ID => (8..=100).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            auth::CLIENT_SECRET => (8..=200).contains(&value.len()) && value.chars().all(|c| c.is_ascii_graphic()),
            _ => return Err(ToolError::validation(format!("Unknown field '{key}'."))),
        };
        if ok {
            Ok(())
        } else {
            Err(ToolError::validation("Client ID and Client Secret must be copied exactly from apps.developer.allegro.pl (no spaces)."))
        }
    }

    fn token_kinds(&self) -> &'static [&'static str] {
        &[auth::REFRESH_TOKEN, auth::ACCESS_TOKEN]
    }

    async fn begin_authorization(&self, ctx: &SourceContext<'_>) -> Result<DeviceAuthorization, ToolError> {
        self.begin(ctx).await
    }

    async fn poll_authorization(&self, ctx: &SourceContext<'_>, device_code: &Secret) -> Result<AuthPoll, ToolError> {
        self.poll(ctx, device_code).await
    }

    async fn test_connection(&self, ctx: &SourceContext<'_>) -> Result<String, ToolError> {
        let me = self.get(ctx, "/me", &[]).await?;
        Ok(format!("Connected as {}.", me["login"].as_str().unwrap_or("?")))
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "get_account",
                description: "Get the connected Allegro seller account: login, e-mail, company and base marketplace.",
                input_schema: schema(json!({}), &[]),
                read_only: true,
            },
            ToolDef {
                name: "list_orders",
                description: "List orders (Allegro checkout forms) of the seller, newest first. Returns summaries; call get_order for buyer, delivery, invoice and line items. \
                              Page with offset: when next_offset is present, call again with offset = next_offset.",
                input_schema: schema(
                    json!({
                        "status": { "type": "string", "enum": ORDER_STATUSES, "description": "Order status. READY_FOR_PROCESSING = paid/confirmed and ready to be handled by the seller." },
                        "fulfillment_status": { "type": "string", "enum": FULFILLMENT_STATUSES, "description": "Seller-side fulfillment status." },
                        "bought_from": { "type": "string", "description": "Purchased at or after: YYYY-MM-DD or RFC 3339 datetime, UTC." },
                        "bought_to": { "type": "string", "description": "Purchased at or before: YYYY-MM-DD (whole day) or RFC 3339 datetime, UTC." },
                        "buyer_login": { "type": "string", "maxLength": 100, "description": "Exact Allegro login of the buyer." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25, "description": "Maximum number of orders to return." },
                        "offset": { "type": "integer", "minimum": 0, "maximum": ORDERS_WINDOW - 1, "default": 0, "description": "Number of orders to skip (limit + offset ≤ 10000)." }
                    }),
                    &[],
                ),
                read_only: true,
            },
            ToolDef {
                name: "get_order",
                description: "Get full details of one Allegro order: statuses, buyer, payment, delivery address and method, invoice data, message to seller and line items.",
                input_schema: schema(json!({ "order_id": { "type": "string", "description": "Order (checkout form) UUID — the id from list_orders." } }), &["order_id"]),
                read_only: true,
            },
            ToolDef {
                name: "list_offers",
                description: "List the seller's own offers with price, stock (available/sold), publication status and visit/watcher stats. Filter by name or status. \
                              Page with offset: when next_offset is present, call again with offset = next_offset.",
                input_schema: schema(
                    json!({
                        "name": { "type": "string", "maxLength": 75, "description": "Offer title contains this text." },
                        "status": { "type": "string", "enum": OFFER_STATUSES, "description": "Publication status." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50, "description": "Maximum number of offers to return." },
                        "offset": { "type": "integer", "minimum": 0, "maximum": 100000, "default": 0, "description": "Number of offers to skip." }
                    }),
                    &[],
                ),
                read_only: true,
            },
            ToolDef {
                name: "get_offer",
                description: "Get details of one of the seller's offers: title, category, price, stock, publication, delivery and linked catalog products. The HTML description is omitted.",
                input_schema: schema(json!({ "offer_id": { "type": "string", "description": "Numeric offer id — the id from list_offers." } }), &["offer_id"]),
                read_only: true,
            },
        ]
    }

    async fn call_tool(&self, ctx: &SourceContext<'_>, tool: &str, args: &Value) -> Result<Value, ToolError> {
        match tool {
            "get_account" => {
                args_object(args, &[])?;
                let me = self.get(ctx, "/me", &[]).await?;
                Ok(json!({ "account": pick(&me, &["id", "login", "email", "company", "baseMarketplace"]) }))
            }
            "list_orders" => {
                let map = args_object(args, &["status", "fulfillment_status", "bought_from", "bought_to", "buyer_login", "limit", "offset"])?;
                let limit = opt_int(map, "limit", 1, 100)?.unwrap_or(25);
                let offset = opt_int(map, "offset", 0, ORDERS_WINDOW - 1)?.unwrap_or(0);
                if limit + offset > ORDERS_WINDOW {
                    return Err(ToolError::validation("limit + offset must not exceed 10000; narrow the date range instead."));
                }
                let (from, to) = (opt_date(map, "bought_from", false)?, opt_date(map, "bought_to", true)?);
                if matches!((from, to), (Some(from), Some(to)) if to < from) {
                    return Err(ToolError::validation("'bought_to' is earlier than 'bought_from'."));
                }
                let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string())];
                let filters = [
                    ("status", opt_enum(map, "status", &ORDER_STATUSES)?.map(String::from)),
                    ("fulfillment.status", opt_enum(map, "fulfillment_status", &FULFILLMENT_STATUSES)?.map(String::from)),
                    ("lineItems.boughtAt.gte", from.and_then(iso)),
                    ("lineItems.boughtAt.lte", to.and_then(iso)),
                    ("buyer.login", opt_str(map, "buyer_login", 100)?.map(String::from)),
                ];
                query.extend(filters.into_iter().filter_map(|(key, value)| Some((key, value?))));

                let response = self.get(ctx, "/order/checkout-forms", &query).await?;
                let orders: Vec<Value> = response["checkoutForms"].as_array().map(|forms| forms.iter().map(order_summary).collect()).unwrap_or_default();
                Ok(paged("orders", orders, offset, &response))
            }
            "get_order" => {
                let map = args_object(args, &["order_id"])?;
                let id = opt_str(map, "order_id", 36)?
                    .filter(|id| is_uuid(id))
                    .ok_or_else(|| ToolError::validation("'order_id' must be the order UUID from list_orders."))?;
                let order = self.get(ctx, &format!("/order/checkout-forms/{id}"), &[]).await?;
                Ok(json!({ "order": order_details(&order) }))
            }
            "list_offers" => {
                let map = args_object(args, &["name", "status", "limit", "offset"])?;
                let limit = opt_int(map, "limit", 1, 200)?.unwrap_or(50);
                let offset = opt_int(map, "offset", 0, 100_000)?.unwrap_or(0);
                let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string())];
                let filters = [
                    ("name", opt_str(map, "name", 75)?.map(String::from)),
                    ("publication.status", opt_enum(map, "status", &OFFER_STATUSES)?.map(String::from)),
                ];
                query.extend(filters.into_iter().filter_map(|(key, value)| Some((key, value?))));

                let response = self.get(ctx, "/sale/offers", &query).await?;
                let fields = ["id", "name", "category", "sellingMode", "stock", "stats", "publication", "external"];
                Ok(paged("offers", pick_each(&response["offers"], &fields), offset, &response))
            }
            "get_offer" => {
                let map = args_object(args, &["offer_id"])?;
                let id = opt_str(map, "offer_id", 20)?
                    .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
                    .ok_or_else(|| ToolError::validation("'offer_id' must be the numeric offer id from list_offers."))?;
                let offer = self.get(ctx, &format!("/sale/product-offers/{id}"), &[]).await?;
                Ok(json!({ "offer": offer_details(&offer) }))
            }
            other => Err(ToolError::validation(format!("Unknown tool '{other}'."))),
        }
    }
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36 && value.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Wspólna koperta list: `count` = liczba pozycji na tej stronie, `totalCount` = wszystkich pasujących.
fn paged(key: &str, items: Vec<Value>, offset: i64, response: &Value) -> Value {
    let returned = items.len() as i64;
    let total = response["totalCount"].as_i64().unwrap_or(offset + returned);
    let next_offset = (offset + returned < total && returned > 0).then_some(offset + returned);
    json!({ key: items, "returned": returned, "total_count": total, "next_offset": next_offset })
}

fn order_summary(order: &Value) -> Value {
    let mut out = pick(order, &["id", "status", "updatedAt"]);
    let extra = [
        ("fulfillment_status", &order["fulfillment"]["status"]),
        ("buyer_login", &order["buyer"]["login"]),
        ("bought_at", &order["lineItems"][0]["boughtAt"]),
        ("total_to_pay", &order["summary"]["totalToPay"]),
        ("payment_type", &order["payment"]["type"]),
        ("paid_at", &order["payment"]["finishedAt"]),
        ("delivery_method", &order["delivery"]["method"]["name"]),
    ];
    out.extend(extra.into_iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.to_string(), value.clone())));
    out.insert("items_count".into(), order["lineItems"].as_array().map_or(0, Vec::len).into());
    Value::Object(out)
}

fn order_details(order: &Value) -> Value {
    let fields = [
        "id",
        "status",
        "fulfillment",
        "buyer",
        "payment",
        "delivery",
        "invoice",
        "surcharges",
        "discounts",
        "summary",
        "messageToSeller",
        "marketplace",
        "updatedAt",
        "revision",
    ];
    let mut out = pick(order, &fields);
    out.insert("lineItems".into(), pick_each(&order["lineItems"], &["id", "offer", "quantity", "originalPrice", "price", "boughtAt"]).into());
    Value::Object(out)
}

fn offer_details(offer: &Value) -> Value {
    let fields = ["id", "name", "language", "category", "sellingMode", "stock", "publication", "external", "delivery", "location", "createdAt", "updatedAt"];
    let mut out = pick(offer, &fields);
    let products: Vec<Value> = offer["productSet"]
        .as_array()
        .map(|set| set.iter().map(|entry| json!({ "product": pick(&entry["product"], &["id", "name"]), "quantity": entry["quantity"]["value"] })).collect())
        .unwrap_or_default();
    out.insert("products".into(), products.into());
    out.insert("images_count".into(), offer["images"].as_array().map_or(0, Vec::len).into());
    Value::Object(out)
}
