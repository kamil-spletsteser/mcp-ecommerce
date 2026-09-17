//! BaseLinker — pełna integracja MVP. Każde narzędzie ma wąski cel; nie ma tu uniwersalnego „wywołaj metodę API”.
//!
//! Użyte metody API: getOrderStatusList, getOrders, getInventories, getInventoryWarehouses,
//! getInventoryProductsList, setOrderStatus, setOrderFields (pole `admin_comments`, maks. 200 znaków).

mod client;
#[cfg(test)]
mod tests;

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use time::OffsetDateTime;

use super::{
    args_object, iso, object_schema as schema, opt_date, opt_int, opt_str, pick, pick_each, req_int, AuthKind, Capability, ErrorCode, FieldSpec, Provider,
    ProviderMeta, SourceContext, ToolDef, ToolError,
};
use client::Client;

pub const TOKEN_KIND: &str = "api_token";
const MAX_ID: i64 = i32::MAX as i64;
/// Limit `getOrders` po stronie BaseLinkera.
const ORDERS_PAGE: usize = 100;
const NOTE_MAX_CHARS: usize = 200;
const DAY: i64 = 86_400;

pub struct BaseLinker {
    client: Client,
}

impl Default for BaseLinker {
    fn default() -> Self {
        // Adres API jest stały; nadpisanie wyłącznie w buildach debug na potrzeby testów E2E z mockiem.
        #[cfg(debug_assertions)]
        if let Ok(url) = std::env::var("ECOMMERCE_MCP_BASELINKER_URL") {
            return Self::with_client(url, Duration::from_secs(30), Duration::from_millis(500));
        }
        Self::with_client(client::API_URL.into(), Duration::from_secs(30), Duration::from_millis(500))
    }
}

impl BaseLinker {
    pub fn with_client(base_url: String, timeout: Duration, backoff: Duration) -> Self {
        Self { client: Client::new(base_url, timeout, backoff) }
    }

    async fn read(&self, ctx: &SourceContext<'_>, method: &'static str, params: Value) -> Result<Value, ToolError> {
        self.client.call(&ctx.secret(TOKEN_KIND)?, method, params, true).await
    }

    async fn write(&self, ctx: &SourceContext<'_>, method: &'static str, params: Value) -> Result<Value, ToolError> {
        self.client.call(&ctx.secret(TOKEN_KIND)?, method, params, false).await
    }

    async fn fetch_order(&self, ctx: &SourceContext<'_>, order_id: i64) -> Result<Value, ToolError> {
        let response = self.read(ctx, "getOrders", json!({ "order_id": order_id, "get_unconfirmed_orders": true })).await?;
        response["orders"]
            .as_array()
            .and_then(|orders| orders.iter().find(|o| int(&o["order_id"]) == order_id))
            .cloned()
            .ok_or_else(|| ToolError::new(ErrorCode::NotFound, format!("Order {order_id} not found.")))
    }
}

#[async_trait]
impl Provider for BaseLinker {
    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "baselinker",
            name: "BaseLinker",
            auth: AuthKind::Fields,
            fields: vec![FieldSpec::new(TOKEN_KIND, true, true, 300)],
            capabilities: vec![
                Capability { label_key: "cap.orders.read", write: false, tools: vec!["list_orders", "get_order"] },
                Capability { label_key: "cap.statuses.read", write: false, tools: vec!["get_order_statuses"] },
                Capability { label_key: "cap.products.read", write: false, tools: vec!["list_inventories", "list_products"] },
                Capability { label_key: "cap.warehouses.read", write: false, tools: vec!["list_warehouses"] },
                Capability { label_key: "cap.orders.update_status", write: true, tools: vec!["update_order_status"] },
                Capability { label_key: "cap.orders.add_note", write: true, tools: vec!["add_order_note"] },
            ],
        }
    }

    fn validate_field(&self, kind: &str, value: &str) -> Result<(), ToolError> {
        if kind != TOKEN_KIND {
            return Err(ToolError::validation(format!("Unknown field '{kind}'.")));
        }
        let ok = (10..=300).contains(&value.len()) && value.chars().all(|c| c.is_ascii_graphic());
        if ok {
            Ok(())
        } else {
            Err(ToolError::validation("The API token must be 10–300 printable characters without spaces."))
        }
    }

    async fn test_connection(&self, ctx: &SourceContext<'_>) -> Result<String, ToolError> {
        let response = self.read(ctx, "getOrderStatusList", json!({})).await?;
        let count = response["statuses"].as_array().map_or(0, Vec::len);
        Ok(format!("Connected. {count} order statuses available."))
    }

    fn tools(&self) -> Vec<ToolDef> {
        let order_id = json!({ "type": "integer", "minimum": 1, "maximum": MAX_ID, "description": "BaseLinker order ID (order_id from list_orders)." });
        vec![
            ToolDef {
                name: "get_order_statuses",
                description: "List all order statuses defined in this BaseLinker account (id, name). Use the ids with list_orders and update_order_status.",
                input_schema: schema(json!({}), &[]),
                read_only: true,
            },
            ToolDef {
                name: "list_orders",
                description: "List confirmed orders, oldest first, starting at date_from (by confirmation date). Returns order summaries; call get_order for full details. \
                              At most 100 orders per call: when has_more is true, call again with date_from = next_date_from. \
                              To see the newest orders, pass a recent date_from (e.g. today or yesterday).",
                input_schema: schema(
                    json!({
                        "date_from": { "type": "string", "description": "Start of range (confirmation date): YYYY-MM-DD or RFC 3339 datetime, UTC. Default: 30 days ago." },
                        "date_to": { "type": "string", "description": "End of range, inclusive: YYYY-MM-DD (whole day) or RFC 3339 datetime, UTC. Range may span at most 366 days." },
                        "status_id": { "type": "integer", "minimum": 1, "maximum": MAX_ID, "description": "Only orders in this status (id from get_order_statuses)." },
                        "email": { "type": "string", "maxLength": 50, "description": "Only orders of the buyer with this exact e-mail address." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": ORDERS_PAGE, "default": 25, "description": "Maximum number of orders to return." }
                    }),
                    &[],
                ),
                read_only: true,
            },
            ToolDef {
                name: "get_order",
                description: "Get full details of one order: status, dates, buyer, delivery and invoice data, payment, comments and ordered products.",
                input_schema: schema(json!({ "order_id": order_id }), &["order_id"]),
                read_only: true,
            },
            ToolDef {
                name: "list_inventories",
                description: "List product catalogs (inventories) of this BaseLinker account. list_products requires an inventory_id from this list.",
                input_schema: schema(json!({}), &[]),
                read_only: true,
            },
            ToolDef {
                name: "list_warehouses",
                description: "List warehouses available in BaseLinker inventories (id, name, type). Stock levels in list_products are keyed by '<warehouse_type>_<warehouse_id>'.",
                input_schema: schema(json!({}), &[]),
                read_only: true,
            },
            ToolDef {
                name: "list_products",
                description: "List products of one catalog with SKU, EAN, name, prices (by price group id) and stock (by warehouse). \
                              Filter by name, SKU or EAN to find specific products. BaseLinker pages hold 1000 products: if has_next_page is true, request page + 1; \
                              if truncated is true, raise limit or narrow the filters.",
                input_schema: schema(
                    json!({
                        "inventory_id": { "type": "integer", "minimum": 1, "maximum": MAX_ID, "description": "Catalog id from list_inventories." },
                        "name": { "type": "string", "maxLength": 200, "description": "Product name contains this text." },
                        "sku": { "type": "string", "maxLength": 50, "description": "Exact SKU." },
                        "ean": { "type": "string", "maxLength": 32, "description": "Exact EAN." },
                        "page": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 1, "description": "BaseLinker result page (1000 products each)." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100, "description": "Maximum number of products to return from this page." }
                    }),
                    &["inventory_id"],
                ),
                read_only: true,
            },
            ToolDef {
                name: "update_order_status",
                description: "WRITE: move one order to another status. Changes data in BaseLinker and may trigger the seller's automations (e-mails, shipments). \
                              Confirm the target status with the user first.",
                input_schema: schema(
                    json!({
                        "order_id": order_id,
                        "status_id": { "type": "integer", "minimum": 1, "maximum": MAX_ID, "description": "Target status id from get_order_statuses." }
                    }),
                    &["order_id", "status_id"],
                ),
                read_only: false,
            },
            ToolDef {
                name: "add_order_note",
                description: "WRITE: append a note to the seller comments (admin_comments) of one order. Existing comments are kept; \
                              BaseLinker limits the whole field to 200 characters, so the call fails if the combined text would be longer.",
                input_schema: schema(
                    json!({
                        "order_id": order_id,
                        "note": { "type": "string", "minLength": 1, "maxLength": NOTE_MAX_CHARS, "description": "Text to append." }
                    }),
                    &["order_id", "note"],
                ),
                read_only: false,
            },
        ]
    }

    async fn call_tool(&self, ctx: &SourceContext<'_>, tool: &str, args: &Value) -> Result<Value, ToolError> {
        match tool {
            "get_order_statuses" => {
                args_object(args, &[])?;
                let response = self.read(ctx, "getOrderStatusList", json!({})).await?;
                Ok(json!({ "statuses": pick_each(&response["statuses"], &["id", "name", "name_for_customer", "color"]) }))
            }
            "list_orders" => self.list_orders(ctx, args).await,
            "get_order" => {
                let map = args_object(args, &["order_id"])?;
                let order = self.fetch_order(ctx, req_int(map, "order_id", 1, MAX_ID)?).await?;
                Ok(json!({ "order": order_details(&order) }))
            }
            "list_inventories" => {
                args_object(args, &[])?;
                let response = self.read(ctx, "getInventories", json!({})).await?;
                let fields = [
                    "inventory_id",
                    "name",
                    "description",
                    "languages",
                    "default_language",
                    "price_groups",
                    "default_price_group",
                    "warehouses",
                    "default_warehouse",
                    "is_default",
                ];
                Ok(json!({ "inventories": pick_each(&response["inventories"], &fields) }))
            }
            "list_warehouses" => {
                args_object(args, &[])?;
                let response = self.read(ctx, "getInventoryWarehouses", json!({})).await?;
                let fields = ["warehouse_type", "warehouse_id", "name", "description", "is_default", "stock_edition", "city", "country"];
                Ok(json!({ "warehouses": pick_each(&response["warehouses"], &fields) }))
            }
            "list_products" => self.list_products(ctx, args).await,
            "update_order_status" => {
                let map = args_object(args, &["order_id", "status_id"])?;
                let order_id = req_int(map, "order_id", 1, MAX_ID)?;
                let status_id = req_int(map, "status_id", 1, MAX_ID)?;
                self.write(ctx, "setOrderStatus", json!({ "order_id": order_id, "status_id": status_id })).await?;
                Ok(json!({ "ok": true, "action": "order_status_updated", "order_id": order_id, "status_id": status_id }))
            }
            "add_order_note" => {
                let map = args_object(args, &["order_id", "note"])?;
                let order_id = req_int(map, "order_id", 1, MAX_ID)?;
                let note = opt_str(map, "note", NOTE_MAX_CHARS)?.ok_or_else(|| ToolError::validation("'note' is required."))?;
                // setOrderFields nadpisuje pole, więc „dodanie” = odczyt + dopisanie + zapis.
                // ponytail: bez blokady między odczytem a zapisem — równoległa edycja tej samej notatki w panelu BL może ją nadpisać.
                let order = self.fetch_order(ctx, order_id).await?;
                let existing = order["admin_comments"].as_str().unwrap_or("").trim();
                let combined = if existing.is_empty() { note.to_string() } else { format!("{existing} | {note}") };
                let length = combined.chars().count();
                if length > NOTE_MAX_CHARS {
                    return Err(ToolError::validation(format!(
                        "Seller comments are limited to {NOTE_MAX_CHARS} characters; existing text plus the note would be {length}. Shorten the note."
                    )));
                }
                self.write(ctx, "setOrderFields", json!({ "order_id": order_id, "admin_comments": combined })).await?;
                Ok(json!({ "ok": true, "action": "order_note_added", "order_id": order_id, "admin_comments": combined }))
            }
            other => Err(ToolError::validation(format!("Unknown tool '{other}'."))),
        }
    }
}

impl BaseLinker {
    async fn list_orders(&self, ctx: &SourceContext<'_>, args: &Value) -> Result<Value, ToolError> {
        let map = args_object(args, &["date_from", "date_to", "status_id", "email", "limit"])?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let date_from = opt_date(map, "date_from", false)?.unwrap_or(now - 30 * DAY);
        let date_to = opt_date(map, "date_to", true)?;
        if date_from > now + DAY {
            return Err(ToolError::validation("'date_from' is in the future."));
        }
        if let Some(to) = date_to {
            if to < date_from {
                return Err(ToolError::validation("'date_to' is earlier than 'date_from'."));
            }
            if to - date_from > 366 * DAY {
                return Err(ToolError::validation("Date range may span at most 366 days."));
            }
        }
        let limit = opt_int(map, "limit", 1, ORDERS_PAGE as i64)?.unwrap_or(25) as usize;

        let mut params = json!({ "date_confirmed_from": date_from });
        if let Some(status_id) = opt_int(map, "status_id", 1, MAX_ID)? {
            params["status_id"] = status_id.into();
        }
        if let Some(email) = opt_str(map, "email", 50)? {
            params["filter_email"] = email.into();
        }

        let response = self.read(ctx, "getOrders", params).await?;
        let upstream = response["orders"].as_array().cloned().unwrap_or_default();
        let upstream_full = upstream.len() >= ORDERS_PAGE;
        let mut in_range: Vec<&Value> = upstream.iter().filter(|o| date_to.is_none_or(|to| int(&o["date_confirmed"]) <= to)).collect();
        let cut_by_date_to = in_range.len() < upstream.len();
        in_range.sort_by_key(|o| int(&o["date_confirmed"]));
        let truncated = in_range.len() > limit;
        in_range.truncate(limit);

        let has_more = truncated || (upstream_full && !cut_by_date_to);
        let next_date_from = in_range.last().filter(|_| has_more).and_then(|o| iso(int(&o["date_confirmed"]) + 1));
        Ok(json!({
            "orders": in_range.iter().map(|o| order_summary(o)).collect::<Vec<_>>(),
            "returned": in_range.len(),
            "has_more": has_more,
            "next_date_from": next_date_from,
        }))
    }

    async fn list_products(&self, ctx: &SourceContext<'_>, args: &Value) -> Result<Value, ToolError> {
        let map = args_object(args, &["inventory_id", "name", "sku", "ean", "page", "limit"])?;
        let page = opt_int(map, "page", 1, 10_000)?.unwrap_or(1);
        let limit = opt_int(map, "limit", 1, 1000)?.unwrap_or(100) as usize;
        let mut params = json!({ "inventory_id": req_int(map, "inventory_id", 1, MAX_ID)?, "page": page });
        for (arg, filter, max_len) in [("name", "filter_name", 200), ("sku", "filter_sku", 50), ("ean", "filter_ean", 32)] {
            if let Some(value) = opt_str(map, arg, max_len)? {
                params[filter] = value.into();
            }
        }

        let response = self.read(ctx, "getInventoryProductsList", params).await?;
        // `products` to obiekt {id: {...}}; pusty wynik BaseLinker (PHP) potrafi zwrócić jako [].
        let mut products: Vec<Value> = match &response["products"] {
            Value::Object(by_id) => by_id.values().cloned().collect(),
            Value::Array(list) => list.clone(),
            _ => vec![],
        };
        products.sort_by_key(|p| int(&p["id"]));
        let upstream_count = products.len();
        products.truncate(limit);
        let fields = ["id", "parent_id", "sku", "ean", "name", "prices", "stock"];
        Ok(json!({
            "products": products.iter().map(|p| Value::Object(pick(p, &fields))).collect::<Vec<_>>(),
            "returned": products.len(),
            "page": page,
            "truncated": upstream_count > limit,
            "has_next_page": upstream_count >= 1000,
        }))
    }
}

/// BaseLinker zwraca liczby raz jako number, raz jako string.
fn int(value: &Value) -> i64 {
    value.as_i64().or_else(|| value.as_str()?.trim().parse().ok()).unwrap_or(0)
}

fn num(value: &Value) -> f64 {
    value.as_f64().or_else(|| value.as_str()?.trim().parse().ok()).unwrap_or(0.0)
}

fn total_gross(order: &Value) -> f64 {
    let products: f64 = order["products"].as_array().map_or(0.0, |items| items.iter().map(|p| num(&p["price_brutto"]) * num(&p["quantity"])).sum());
    ((products + num(&order["delivery_price"])) * 100.0).round() / 100.0
}

fn with_dates(mut out: Map<String, Value>, order: &Value) -> Map<String, Value> {
    for field in ["date_add", "date_confirmed", "date_in_status"] {
        if let Some(date) = iso(int(&order[field])) {
            out.insert(field.into(), date.into());
        }
    }
    out.insert("total_gross".into(), total_gross(order).into());
    out
}

fn order_summary(order: &Value) -> Value {
    let mut out = pick(order, &["order_id", "order_status_id", "order_source", "delivery_fullname", "email", "currency", "payment_done", "delivery_method"]);
    out.insert("products_count".into(), order["products"].as_array().map_or(0, Vec::len).into());
    Value::Object(with_dates(out, order))
}

fn order_details(order: &Value) -> Value {
    let mut out = pick(
        order,
        &[
            "order_id",
            "shop_order_id",
            "external_order_id",
            "order_source",
            "order_status_id",
            "confirmed",
            "currency",
            "payment_method",
            "payment_method_cod",
            "payment_done",
            "user_comments",
            "admin_comments",
            "email",
            "phone",
            "user_login",
            "delivery_method",
            "delivery_price",
            "delivery_package_module",
            "delivery_package_nr",
            "delivery_fullname",
            "delivery_company",
            "delivery_address",
            "delivery_postcode",
            "delivery_city",
            "delivery_state",
            "delivery_country_code",
            "delivery_point_id",
            "delivery_point_name",
            "delivery_point_address",
            "delivery_point_postcode",
            "delivery_point_city",
            "invoice_fullname",
            "invoice_company",
            "invoice_nip",
            "invoice_address",
            "invoice_postcode",
            "invoice_city",
            "invoice_state",
            "invoice_country_code",
            "want_invoice",
            "extra_field_1",
            "extra_field_2",
            "order_page",
        ],
    );
    let product_fields = [
        "order_product_id",
        "product_id",
        "variant_id",
        "name",
        "sku",
        "ean",
        "attributes",
        "location",
        "warehouse_id",
        "price_brutto",
        "tax_rate",
        "quantity",
        "weight",
    ];
    out.insert("products".into(), pick_each(&order["products"], &product_fields).into());
    Value::Object(with_dates(out, order))
}
