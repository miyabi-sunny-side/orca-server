//! MCP tools call the existing in-process API, including its validation and locks.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

type ApiResult = Result<Value, Value>;
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    q: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Save<T> {
    /// Omit for creation; use an ID returned by a read for replacement.
    id: Option<String>,
    data: T,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProductEntry<T> {
    product_id: String,
    /// Omit for creation; use a color/setting ID for replacement.
    id: Option<String>,
    data: T,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlateSave {
    /// Omit for creation. An update requires plate.version from `plate_get`.
    id: Option<String>,
    /// Full replacement, including every model and all desired conditions.
    plate: crate::plates::Edit,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Machine {
    machine: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MaterialProfiles {
    product_id: String,
    machine: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Printer {
    printer_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Assign {
    printer_id: String,
    slot_id: String,
    revision: i64,
    /// Color ID, or null to clear the mapping.
    #[schemars(required)]
    #[serde(deserialize_with = "nullable_id")]
    filament_id: Option<String>,
}
fn nullable_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(d)
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Resolve {
    printer_id: String,
    filament_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Prioritize {
    printer_id: String,
    priority: crate::ams::Priority,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Admission {
    printer_id: String,
    plate_id: String,
}

#[derive(Clone)]
struct Tools {
    api: Router,
    tool_router: ToolRouter<Self>,
}
fn failure(status: u16, message: &str) -> Value {
    json!({"status":status,"error":message})
}
fn answer(result: ApiResult) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(json!({"data":value})),
        Err(error) => CallToolResult::structured_error(error),
    }
}
fn plate_answer(result: ApiResult) -> CallToolResult {
    match result {
        Ok(plate) => CallToolResult::structured(
            json!({"ui_path":format!("/plates/{}",plate["id"].as_str().expect("plate ID")),"data":plate}),
        ),
        Err(error) => CallToolResult::structured_error(error),
    }
}
impl Tools {
    async fn api(
        &self,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> ApiResult {
        // Only fixed routes reach this function. Encode each argument as a single segment.
        if path
            .iter()
            .any(|part| part.is_empty() || matches!(*part, "." | ".."))
        {
            return Err(failure(400, "Invalid ID"));
        }
        let mut url = reqwest::Url::parse("http://in-process/").expect("fixed URL");
        url.path_segments_mut()
            .expect("HTTP URL")
            .clear()
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let uri = url
            .as_str()
            .strip_prefix("http://in-process")
            .expect("fixed API origin");
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("encoded API request");
        let response = self
            .api
            .clone()
            .oneshot(request)
            .await
            .expect("infallible router");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|_| {
                failure(
                    500,
                    "API response could not be read; inspect saved state before retrying",
                )
            })?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!({"error":String::from_utf8_lossy(&bytes)}))
        };
        if status.is_success() {
            Ok(value)
        } else {
            Err(failure(
                status.as_u16(),
                value["error"].as_str().unwrap_or("API request failed"),
            ))
        }
    }
    async fn machine_options(&self, machine: Option<&str>) -> ApiResult {
        let printers = self
            .api(Method::GET, &["api", "printers"], &[], None)
            .await?;
        let machines: std::collections::BTreeSet<_> = printers
            .as_array()
            .expect("printer list")
            .iter()
            .filter_map(|p| p["machine_profile_key"].as_str())
            .collect();
        let profiles = if let Some(machine) = machine {
            if !machines.contains(machine) {
                return Err(failure(409, "Register a matching machine and nozzle first"));
            }
            self.api(
                Method::GET,
                &["api", "slicer", "profiles"],
                &[("machine", machine)],
                None,
            )
            .await?
        } else {
            Value::Null
        };
        Ok(json!({"machines":machines,"profiles":profiles}))
    }
}
#[tool_router]
impl Tools {
    #[tool(
        description = "List/search currently published SCAD STL references. q is optional fuzzy search. Save a returned path; this does not publish SCAD or download STL.",
        annotations(read_only_hint = true)
    )]
    async fn scad_models(&self, Parameters(a): Parameters<Search>) -> CallToolResult {
        answer(
            self.api(
                Method::GET,
                &["api", "scad", "models"],
                &[("q", &a.q)],
                None,
            )
            .await,
        )
    }
    #[tool(
        description = "List/search saved plates and their current composition, version and nullable conditions. Reconcile an uncertain create here before retrying.",
        annotations(read_only_hint = true)
    )]
    async fn plate_list(&self, Parameters(a): Parameters<Search>) -> CallToolResult {
        answer(
            self.api(Method::GET, &["api", "plates"], &[("q", &a.q)], None)
                .await,
        )
    }
    #[tool(
        description = "Read a saved plate, version, composition and management UI path relative to this server. Saving is separate from queue admission.",
        annotations(read_only_hint = true)
    )]
    async fn plate_get(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        plate_answer(
            self.api(Method::GET, &["api", "plates", &a.id], &[], None)
                .await,
        )
    }
    #[tool(
        description = "Create or replace a saved plate. Omit id for create; for update read first and supply current plate.version and ALL models. Preserve uploaded model IDs. SCAD source must exist. On create, omitted/null conditions use the saved printer defaults and first usable material in its synchronized AMS; explicit values win. On update, omitted/null fields clear conditions; send values to preserve them. Never invent conditions. Does not enqueue or start printing. On an uncertain response, use plate_list/get before any new create."
    )]
    async fn plate_save(&self, Parameters(a): Parameters<PlateSave>) -> CallToolResult {
        let result = if let Some(id) = a.id {
            self.api(
                Method::PUT,
                &["api", "plates", &id],
                &[],
                Some(json!(a.plate)),
            )
            .await
        } else {
            self.api(
                Method::POST,
                &["api", "plates", "import"],
                &[],
                Some(json!(a.plate)),
            )
            .await
        };
        plate_answer(result)
    }
    #[tool(
        description = "List filament products with all colors and shared machine settings. Color IDs are the filament_id used by plates and AMS. Reconcile uncertain creates using this list.",
        annotations(read_only_hint = true)
    )]
    async fn filament_products(&self) -> CallToolResult {
        answer(
            self.api(Method::GET, &["api", "filament-products"], &[], None)
                .await,
        )
    }
    #[tool(
        description = "Read one product, colors, common settings and resolved temperatures. Never copy a setting per color.",
        annotations(read_only_hint = true)
    )]
    async fn filament_product(&self, Parameters(a): Parameters<Id>) -> CallToolResult {
        answer(
            self.api(Method::GET, &["api", "filament-products", &a.id], &[], None)
                .await,
        )
    }
    #[tool(
        description = "Create a product (omit id) or replace its common name/vendor/material/Bambu ID (supply id). Read existing products first; after an uncertain write, read before retrying."
    )]
    async fn filament_product_save(
        &self,
        Parameters(a): Parameters<Save<crate::products::ProductData>>,
    ) -> CallToolResult {
        answer(if let Some(id) = a.id {
            self.api(
                Method::PUT,
                &["api", "filament-products", &id],
                &[],
                Some(json!(a.data)),
            )
            .await
        } else {
            self.api(
                Method::POST,
                &["api", "filament-products"],
                &[],
                Some(json!(a.data)),
            )
            .await
        })
    }
    #[tool(
        description = "Add a color to an existing product (omit id), or replace its color name/exact eight-digit RGBA (supply color id). Common temperature settings are automatically shared. Read the product after an uncertain result before retrying."
    )]
    async fn filament_color_save(
        &self,
        Parameters(a): Parameters<ProductEntry<crate::products::ColorData>>,
    ) -> CallToolResult {
        answer(if let Some(id) = a.id {
            self.api(
                Method::PUT,
                &["api", "filament-products", &a.product_id, "colors", &id],
                &[],
                Some(json!(a.data)),
            )
            .await
        } else {
            self.api(
                Method::POST,
                &["api", "filament-products", &a.product_id, "colors"],
                &[],
                Some(json!(a.data)),
            )
            .await
        })
    }
    #[tool(
        description = "Create (omit id) or replace (supply setting id) one product's shared machine/nozzle setting and explicit temperature overrides. All colors share it. Read available profiles first. No arbitrary G-code keys. Read back after an uncertain write."
    )]
    async fn filament_setting_save(
        &self,
        Parameters(a): Parameters<ProductEntry<crate::filament::SettingData>>,
    ) -> CallToolResult {
        answer(if let Some(id) = a.id {
            self.api(
                Method::PUT,
                &["api", "filament-products", &a.product_id, "settings", &id],
                &[],
                Some(json!(a.data)),
            )
            .await
        } else {
            self.api(
                Method::POST,
                &["api", "filament-products", &a.product_id, "settings"],
                &[],
                Some(json!(a.data)),
            )
            .await
        })
    }
    #[tool(
        description = "Read compatible material base profiles/temperatures for a product and explicit machine/nozzle key.",
        annotations(read_only_hint = true)
    )]
    async fn filament_profiles(
        &self,
        Parameters(a): Parameters<MaterialProfiles>,
    ) -> CallToolResult {
        answer(
            self.api(
                Method::GET,
                &["api", "filament-products", &a.product_id, "profiles"],
                &[("machine", &a.machine)],
                None,
            )
            .await,
        )
    }
    #[tool(
        description = "Search color materials by product/vendor/material/color. Returned IDs are suitable for plate conditions and AMS mapping.",
        annotations(read_only_hint = true)
    )]
    async fn filaments_search(&self, Parameters(a): Parameters<Search>) -> CallToolResult {
        answer(
            self.api(Method::GET, &["api", "filaments"], &[("q", &a.q)], None)
                .await,
        )
    }
    #[tool(
        description = "Read registered physical printers and reported status. A required machine/nozzle profile is not a physical printer ID.",
        annotations(read_only_hint = true)
    )]
    async fn printers(&self) -> CallToolResult {
        answer(self.api(Method::GET, &["api", "printers"], &[], None).await)
    }
    #[tool(
        description = "Read one printer's AMS observations, mapping IDs/revisions, current flag and priority groups. Unconfirmed observations are not current inventory.",
        annotations(read_only_hint = true)
    )]
    async fn ams_get(&self, Parameters(a): Parameters<Printer>) -> CallToolResult {
        answer(
            self.api(
                Method::GET,
                &["api", "printers", &a.printer_id, "ams"],
                &[],
                None,
            )
            .await,
        )
    }
    #[tool(
        description = "Assign an existing color ID to an AMS slot, or explicitly pass filament_id:null to clear it. Requires the current observed revision; conflicts require rereading ams_get. Does not move filament or start printing."
    )]
    async fn ams_assign(&self, Parameters(a): Parameters<Assign>) -> CallToolResult {
        answer(
            self.api(
                Method::PUT,
                &["api", "printers", &a.printer_id, "ams", &a.slot_id],
                &[],
                Some(json!({"revision":a.revision,"filament_id":a.filament_id})),
            )
            .await,
        )
    }
    #[tool(
        description = "Read the usable slots and preferred initial slot for one material on one printer, using the same resolver as queue admission.",
        annotations(read_only_hint = true)
    )]
    async fn ams_resolve(&self, Parameters(a): Parameters<Resolve>) -> CallToolResult {
        answer(
            self.api(
                Method::GET,
                &["api", "printers", &a.printer_id, "ams", "resolve"],
                &[("filament_id", &a.filament_id)],
                None,
            )
            .await,
        )
    }
    #[tool(
        description = "Replace the complete priority group for an exact product/color on one printer. Supply every current slot ID and revision in the desired order. Stale/missing/duplicate members fail atomically. Does not change native refill or start printing."
    )]
    async fn ams_prioritize(&self, Parameters(a): Parameters<Prioritize>) -> CallToolResult {
        answer(
            self.api(
                Method::PUT,
                &["api", "printers", &a.printer_id, "ams", "priority"],
                &[],
                Some(json!(a.priority)),
            )
            .await,
        )
    }
    #[tool(
        description = "Read deduplicated owned machine/nozzle keys. Pass one explicit returned machine to read compatible process/material/bed choices. Defaults in the profile are suggestions, never authorization to fill missing plate conditions.",
        annotations(read_only_hint = true)
    )]
    async fn plate_options(&self, Parameters(a): Parameters<Machine>) -> CallToolResult {
        answer(self.machine_options(a.machine.as_deref()).await)
    }
    #[tool(
        description = "Read queue admission for a saved plate on an explicit physical printer, including its current version, allowed flag and reason. Saving a plate does not imply it can be queued. This never enqueues or starts printing.",
        annotations(read_only_hint = true)
    )]
    async fn plate_admission(&self, Parameters(a): Parameters<Admission>) -> CallToolResult {
        answer(
            self.api(
                Method::GET,
                &["api", "queue"],
                &[("printer_id", &a.printer_id), ("plate_id", &a.plate_id)],
                None,
            )
            .await
            .map(|value| value["admission"].clone()),
        )
    }
}
#[tool_handler(router=self.tool_router)]
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        )
    }
}
pub(crate) fn mount(api: Router) -> Router {
    let handler = Tools {
        api: api.clone(),
        tool_router: Tools::tool_router(),
    };
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        // The service already uses a trusted LAN/Tailscale boundary and same-origin writes.
        StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_max_request_body_bytes(256 * 1024),
    );
    api.nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(crate::plate_api::same_origin))
}
