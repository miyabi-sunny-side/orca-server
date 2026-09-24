mod common;
use axum::{Json, Router, routing::get};
use rmcp::{
    RoleClient, ServiceExt, model::CallToolRequestParams, service::RunningService,
    transport::StreamableHttpClientTransport,
};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;
struct Server {
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(app: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { base, task }
}
async fn call(client: &Client, name: &str, args: Value, error: bool) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        result.is_error.unwrap_or(false),
        error,
        "{name}: {result:?}"
    );
    result.structured_content.unwrap()
}
async fn read(base: &str, path: &str) -> Value {
    let response = reqwest::get(format!("{base}{path}")).await.unwrap();
    assert!(response.status().is_success());
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep the observed create/read/update/failure sequence together.
async fn client_saves_references_and_shares_the_rest_validation() {
    let source = serve(Router::new().route(
        "/api/models",
        get(|| async { Json(json!(["Gridfinity/10mm.stl"])) }),
    ))
    .await;
    let root = tempfile::tempdir().unwrap();
    let store = orca_server::plates::Store::open(root.path()).unwrap();
    let scad = orca_server::scad::Source::new(&source.base).unwrap();
    let api = orca_server::registry::router(root.path(), store, None, Some(scad)).unwrap();
    let server = serve(orca_server::with_mcp(api)).await;
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "{}/mcp",
            server.base
        )))
        .await
        .unwrap();
    assert_eq!(
        read(&server.base, "/api/default-settings").await["reason"],
        "printer"
    );
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "plate_save"));
    assert!(
        client
            .call_tool(CallToolRequestParams::new("print_start"))
            .await
            .is_err()
    );
    assert_eq!(
        call(&client, "scad_models", json!({"q":"grid10"}), false).await["data"],
        json!(["Gridfinity/10mm.stl"])
    );
    let mut edit = json!({"name":"Gridfinity ×10","models":[{"name":"Gridfinity/10mm.stl","source":"Gridfinity/10mm.stl","quantity":10}]});
    let saved = call(&client, "plate_save", json!({"plate":edit}), false).await;
    let id = saved["data"]["id"].as_str().unwrap();
    assert_eq!(saved["ui_path"], format!("/plates/{id}"));
    let path = format!("/api/plates/{id}");
    let rest = read(&server.base, &path).await;
    assert_eq!(rest, saved["data"]);
    assert_eq!(rest["models"][0]["quantity"], 10);
    for key in [
        "required_machine_profile_key",
        "filament_id",
        "process_profile_key",
        "bed_type",
    ] {
        assert!(rest["conditions"][key].is_null());
    }
    assert_eq!(rest["conditions"]["sparse_infill_pattern"], "adaptivecubic");
    assert_eq!(rest["conditions"]["sparse_infill_density"], 15.0);
    assert_eq!(rest["conditions"]["wall_loops"], 2);
    assert_eq!(
        call(&client, "plate_get", json!({"id":id}), false).await["data"],
        rest
    );
    assert_eq!(
        call(&client, "plate_list", json!({"q":"Gridfinity"}), false).await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    edit["version"] = rest["version"].clone();
    edit["models"] = rest["models"].clone();
    edit["name"] = json!("Ten saved bins");
    let updated =
        call(&client, "plate_save", json!({"id":id,"plate":edit}), false).await["data"].clone();
    assert_eq!(updated["name"], "Ten saved bins");
    let stale = call(&client, "plate_save", json!({"id":id,"plate":edit}), true).await;
    assert_eq!(stale["status"], 409);
    edit["version"] = updated["version"].clone();
    edit["models"][0]["source"] = json!("missing.stl");
    assert_eq!(
        call(&client, "plate_save", json!({"id":id,"plate":edit}), true).await["status"],
        400
    );
    edit["models"] = updated["models"].clone();
    edit["conditions"] = json!({"filament_id":"unknown"});
    assert_eq!(
        call(&client, "plate_save", json!({"id":id,"plate":edit}), true).await["status"],
        404
    );
    assert_eq!(read(&server.base, &path).await, updated);
    for id in ["..", "../queue", "/api/queue?printer_id=p1"] {
        call(&client, "plate_get", json!({"id":id}), true).await;
    }
    let bad = client
        .call_tool(
            CallToolRequestParams::new("plate_get")
                .with_arguments(json!({"id":id,"start":true}).as_object().unwrap().clone()),
        )
        .await;
    assert!(bad.is_err() || bad.unwrap().is_error == Some(true));
    let cross = reqwest::Client::new()
        .post(format!("{}/mcp", server.base))
        .header("origin", "http://unrelated.test")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403);
    let product=call(&client,"filament_product_save",json!({"data":{"name":"PLA Matte","vendor":"Bambu Lab","material":"PLA","bambu_filament_id":"GFA01"}}),false).await["data"].clone();
    let pid = product["id"].as_str().unwrap();
    let color = call(
        &client,
        "filament_color_save",
        json!({"product_id":pid,"data":{"name":"Yellow","color":"FFFF00FF"}}),
        false,
    )
    .await["data"]
        .clone();
    assert_eq!(
        read(&server.base, &format!("/api/filament-products/{pid}")).await["colors"],
        json!([color])
    );
    assert!(
        call(&client, "plate_options", json!({}), false).await["data"]["machines"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(
            &client,
            "plate_options",
            json!({"machine":"unregistered"}),
            true
        )
        .await["status"],
        409
    );
    assert_eq!(read(&server.base, "/api/queue").await["waiting"], json!([]));
    client.cancel().await.unwrap();
}

#[test]
#[allow(clippy::too_many_lines)] // One client follows shared settings through both AMS slots.
fn shared_materials_ams_and_admission() {
    let mut rig = common::Rig::new("mcp-materials");
    rig.launch();
    rig.seed();
    let base = rig.base.clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {

    assert!(base.starts_with("http://127.0.0.1:"));
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "{base}/mcp"
        )))
        .await
        .unwrap();
    let machine = "Bambu Lab P1S 0.4 nozzle";
    let process = "0.20mm Standard @BBL X1C";
    let profile = "Generic PLA High Speed @BBL X1C";
    let printers = call(&client, "printers", json!({}), false).await["data"].clone();
    assert_eq!(printers[0]["id"], "p1");
    assert_eq!(
        call(&client, "plate_options", json!({}), false).await["data"]["machines"],
        json!([machine])
    );
    let options = call(&client, "plate_options", json!({"machine":machine}), false).await;
    assert!(
        options["data"]["profiles"]["processes"]
            .as_array()
            .unwrap()
            .contains(&json!(process))
    );
    assert_eq!(
        call(
            &client,
            "plate_options",
            json!({"machine":"Bambu Lab A1 mini 0.2 nozzle"}),
            true
        )
        .await["status"],
        409
    );
    let mut data = json!({"name":"MCP PLA Matte","vendor":"Bambu Lab","material":"PLA","bambu_filament_id":"GFA01"});
    let product = call(
        &client,
        "filament_product_save",
        json!({"data":data}),
        false,
    )
    .await["data"]
        .clone();
    let pid = product["id"].as_str().unwrap();
    data["name"] = json!("MCP shared PLA");
    call(
        &client,
        "filament_product_save",
        json!({"id":pid,"data":data}),
        false,
    )
    .await;
    let choices = call(
        &client,
        "filament_profiles",
        json!({"product_id":pid,"machine":machine}),
        false,
    )
    .await["data"]
        .clone();
    assert!(
        choices
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["key"] == profile)
    );
    let mut setting = json!({"machine_profile_key":machine,"base_profile_key":profile,"overrides_json":{"nozzle_temperature":215}});
    let common = call(
        &client,
        "filament_setting_save",
        json!({"product_id":pid,"data":setting}),
        false,
    )
    .await["data"]
        .clone();
    let yellow = call(
        &client,
        "filament_color_save",
        json!({"product_id":pid,"data":{"name":"Yellow","color":"FFFF00FF"}}),
        false,
    )
    .await["data"]
        .clone();
    let black = call(
        &client,
        "filament_color_save",
        json!({"product_id":pid,"data":{"name":"Black","color":"000000FF"}}),
        false,
    )
    .await["data"]
        .clone();
    call(
        &client,
        "filament_color_save",
        json!({"product_id":pid,"id":yellow["id"],"data":{"name":"MCP Yellow","color":"FFFF00FF"}}),
        false,
    )
    .await;
    setting["overrides_json"]["nozzle_temperature"] = json!(218);
    call(
        &client,
        "filament_setting_save",
        json!({"product_id":pid,"id":common["id"],"data":setting}),
        false,
    )
    .await;
    setting["overrides_json"]["nozzle_temperature"] = json!(999);
    assert_eq!(
        call(
            &client,
            "filament_setting_save",
            json!({"product_id":pid,"id":common["id"],"data":setting}),
            true
        )
        .await["status"],
        400
    );
    for color in [&yellow, &black] {
        let material = read(
            &base,
            &format!("/api/filaments/{}", color["id"].as_str().unwrap()),
        )
        .await;
        assert_eq!(
            material["settings"][0]["resolved"]["nozzle_temperature"],
            "218"
        );
        assert_eq!(material["settings"][0]["id"], common["id"]);
    }
    assert_eq!(
        call(&client, "filament_product", json!({"id":pid}), false).await["data"],
        read(&base, &format!("/api/filament-products/{pid}")).await
    );
    assert!(
        call(&client, "filament_products", json!({}), false).await["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == pid)
    );
    assert_eq!(
        call(
            &client,
            "filaments_search",
            json!({"q":"MCP Yellow"}),
            false
        )
        .await["data"][0]["id"],
        yellow["id"]
    );
    let mut edit = json!({"name":"MCP Gridfinity ×10","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":10}]});
    let mut plate = call(&client, "plate_save", json!({"plate":edit}), false).await["data"].clone();
    let plate_id = plate["id"].clone();
    let admission = json!({"printer_id":"p1","plate_id":plate_id});
    assert_eq!(
        call(&client, "plate_admission", admission.clone(), false).await["data"]["allowed"],
        true
    );
    assert_eq!(
        plate["conditions"],
        read(&base, "/api/default-settings").await["conditions"]
    );
    edit["version"] = plate["version"].clone();
    edit["conditions"] = json!({"required_machine_profile_key":machine,"filament_id":yellow["id"],"process_profile_key":process,"bed_type":"Textured PEI Plate"});
    plate = call(
        &client,
        "plate_save",
        json!({"id":plate_id,"plate":edit}),
        false,
    )
    .await["data"]
        .clone();
    assert_eq!(
        call(&client, "plate_admission", admission.clone(), false).await["data"]["allowed"],
        false
    );
    let inventory =
        call(&client, "ams_get", json!({"printer_id":"p1"}), false).await["data"].clone();
    for slot in inventory["slots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["reported"]["present"] == true)
    {
        let args = json!({"printer_id":"p1","slot_id":slot["id"],"revision":slot["revision"],"filament_id":yellow["id"]});
        call(&client, "ams_assign", args.clone(), false).await;
        assert_eq!(call(&client, "ams_assign", args, true).await["status"], 409);
    }
    let inventory =
        call(&client, "ams_get", json!({"printer_id":"p1"}), false).await["data"].clone();
    assert_eq!(inventory, read(&base, "/api/printers/p1/ams").await);
    let resolved = call(
        &client,
        "ams_resolve",
        json!({"printer_id":"p1","filament_id":yellow["id"]}),
        false,
    )
    .await["data"]
        .clone();
    let mut order: Vec<_> = resolved["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| json!({"id":s["id"],"revision":s["revision"]}))
        .collect();
    assert_eq!(order.len(), 2);
    order.reverse();
    let priority = json!({"printer_id":"p1","priority":{"filament_id":yellow["id"],"order":order}});
    call(&client, "ams_prioritize", priority.clone(), false).await;
    assert_eq!(
        call(&client, "ams_prioritize", priority, true).await["status"],
        409
    );
    let resolved = call(
        &client,
        "ams_resolve",
        json!({"printer_id":"p1","filament_id":yellow["id"]}),
        false,
    )
    .await["data"]
        .clone();
    assert_eq!(resolved["preferred_slot"]["id"], order[0]["id"]);
    let accepted = call(&client, "plate_admission", admission, false).await["data"].clone();
    assert_eq!(accepted["allowed"], true);
    assert_eq!(accepted["plate_version"], plate["version"]);
    let slot = &resolved["candidates"][0];
    let missing = client
        .call_tool(
            CallToolRequestParams::new("ams_assign").with_arguments(
                json!({"printer_id":"p1","slot_id":slot["id"],"revision":slot["revision"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(missing.is_err() || missing.unwrap().is_error == Some(true));
    assert_eq!(call(&client,"ams_assign",json!({"printer_id":"p1","slot_id":slot["id"],"revision":slot["revision"],"filament_id":"unknown"}),true).await["status"],404);
    call(&client,"ams_assign",json!({"printer_id":"p1","slot_id":slot["id"],"revision":slot["revision"],"filament_id":null}),false).await;
    let rest = read(&base, "/api/printers/p1/ams").await;
    assert!(
        rest["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == slot["id"])
            .unwrap()["filament_id"]
            .is_null()
    );
    assert_eq!(
        read(&base, "/api/queue?printer_id=p1").await["waiting"],
        json!([])
    );
    client.cancel().await.unwrap();
    });
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}

#[test]
fn creation_defaults_match_rest() {
    check_creation_defaults(false);
    check_creation_defaults(true);
}
fn check_creation_defaults(custom: bool) {
    let mut rig = common::Rig::new("mcp-defaults");
    rig.launch();
    rig.seed();
    if custom {
        rig.put("/api/default-settings",&json!({"default_printer_id":"p1","sparse_infill_pattern":"gyroid","sparse_infill_density":22.5,"wall_loops":4}),204);
    }
    let base = rig.base.clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {

    assert!(base.starts_with("http://127.0.0.1:"));
    let client = ()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "{base}/mcp"
        )))
        .await
        .unwrap();
    let defaults = read(&base, "/api/default-settings").await["conditions"].clone();
    assert!(defaults.as_object().unwrap().iter().all(|(key, value)| {
        if key == "support_interface_filament_id" {
            value.is_null()
        } else {
            !value.is_null()
        }
    }));
    let mut edit = json!({"name":"MCP defaults","models":[{"name":"parts/cube.stl","source":"parts/cube.stl","quantity":1}]});
    for nulls in [false, true] {
        if nulls {
            edit["conditions"] = json!({"required_machine_profile_key":null,"filament_id":null,"process_profile_key":null,"bed_type":null});
        }
        let plate = call(&client, "plate_save", json!({"plate":edit}), false).await["data"].clone();
        assert_eq!(plate["conditions"], defaults);
        let id = plate["id"].as_str().unwrap();
        assert_eq!(read(&base, &format!("/api/plates/{id}")).await, plate);
        let mut update = edit.clone();
        update["version"] = plate["version"].clone();
        update["models"] = plate["models"].clone();
        let cleared = call(
            &client,
            "plate_save",
            json!({"id":id,"plate":update}),
            false,
        )
        .await["data"]
            .clone();
        assert!(
            cleared["conditions"]
                .as_object()
                .unwrap()
                .iter()
                .all(
                    |(key, value)| if matches!(key.as_str(), "brim_enabled" | "support_enabled") {
                        value == false
                    } else {
                        value.is_null()
                    }
                )
        );
    }
    edit["conditions"] =
        json!({"sparse_infill_pattern":"gyroid","sparse_infill_density":0,"wall_loops":4});
    let explicit = call(&client, "plate_save", json!({"plate":edit}), false).await["data"].clone();
    assert_eq!(explicit["conditions"]["sparse_infill_pattern"], "gyroid");
    assert_eq!(explicit["conditions"]["sparse_infill_density"], 0.0);
    assert_eq!(explicit["conditions"]["wall_loops"], 4);
    assert_eq!(explicit["conditions"]["brim_enabled"], false);
    let id = explicit["id"].as_str().unwrap();
    let mut current = explicit.clone();
    for enabled in [true, false] {
        let mut update = current.clone();
        update.as_object_mut().unwrap().remove("id");
        update["conditions"]["brim_enabled"] = json!(enabled);
        current = call(
            &client,
            "plate_save",
            json!({"id":id,"plate":update}),
            false,
        )
        .await["data"]
            .clone();
        assert_eq!(current["conditions"]["brim_enabled"], enabled);
        assert_eq!(read(&base, &format!("/api/plates/{id}")).await, current);
        assert_eq!(current["models"], explicit["models"]);
        assert_eq!(current["conditions"]["wall_loops"], 4);
    }
    client.cancel().await.unwrap();
    });
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}

async fn wait_value(base: &str, path: &str, pointer: &str, expected: Value) {
    for _ in 0..1500 {
        if read(base, path).await.pointer(pointer) == Some(&expected) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{path}{pointer} did not become {expected}");
}
async fn control(base: &str, value: Value) {
    assert!(
        reqwest::Client::new()
            .post(base)
            .json(&value)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}
fn continuation(q: &Value) -> Value {
    json!({"printer_id":"p1","epoch":q["epoch"],"generation":q["generation"],"request_id":q["request_id"],
        "next_job":q["waiting"][0]["id"],"removed_job":q["current"]["id"]})
}

#[test]
fn stopped_queue_retry_shares_rest_guards_and_replay_detection() {
    let mut rig = common::Rig::new("mcp-retry");
    rig.launch();
    rig.seed();
    let job = rig.add(3);
    rig.next(&job, 200);
    common::until(|| rig.broker.prints().len() == 1, 12);
    rig.report("RUNNING");
    rig.phase("printing");
    rig.report("FAILED");
    rig.phase("needs_attention");
    let controller = rig.control();
    let base = rig.base.clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client = ().serve(StreamableHttpClientTransport::from_uri(format!("{base}/mcp"))).await.unwrap();
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        assert_eq!(q["allowed"]["retry"], true);
        let args = json!({"printer_id":"p1", "epoch":q["epoch"], "generation":q["generation"], "request_id":q["request_id"], "expected_job":q["current"]["id"]});
        let mut wrong = args.clone(); wrong["expected_job"] = json!("another-job");
        assert_eq!(call(&client, "queue_retry", wrong, true).await["status"], 409);
        let mut stale = args.clone(); stale["epoch"] = json!("old-epoch");
        assert_eq!(call(&client, "queue_retry", stale, true).await["status"], 409);
        let mut stale = args.clone(); stale["generation"] = json!(q["generation"].as_i64().unwrap() - 1);
        assert_eq!(call(&client, "queue_retry", stale, true).await["status"], 409);
        let (one, two) = tokio::join!(call(&client, "queue_retry", args.clone(), false), call(&client, "queue_retry", args.clone(), false));
        assert_eq!(one["data"]["current"]["id"], q["current"]["id"]);
        assert_eq!(two["data"]["current"]["attempt_id"], one["data"]["current"]["attempt_id"]);
        assert_ne!(one["data"]["current"]["attempt_id"], q["current"]["attempt_id"]);
        wait_value(&controller.base, "", "/count", json!(2)).await;
        call(&client, "queue_retry", args, false).await;
        assert_eq!(read(&controller.base, "").await["count"], 2);
        control(&controller.base, json!({"state":"RUNNING"})).await;
        wait_value(&base, "/api/queue", "/current/state", json!("printing")).await;
        let current = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        let busy = json!({"printer_id":"p1", "epoch":current["epoch"], "generation":current["generation"], "request_id":current["request_id"], "expected_job":current["current"]["id"]});
        assert_eq!(call(&client, "queue_retry", busy, true).await["status"], 409);
        assert_eq!(read(&controller.base, "").await["count"], 2);
        client.cancel().await.unwrap();
    });
}
#[test]
#[allow(clippy::too_many_lines)] // Observe one complete two-print cycle and its rejected/replayed requests.
fn queue_continuation() {
    let mut rig = common::Rig::new("mcp-queue");
    rig.launch();
    rig.seed();
    let first = rig.add(3);
    let second = rig.add(0);
    rig.ready(&first);
    rig.ready(&second);
    let controller = rig.control();
    let peer = controller.base.clone();
    let base = rig.base.clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "{base}/mcp"
            )))
            .await
            .unwrap();
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        let rest = read(&base, "/api/queue?printer_id=p1").await;
        assert_eq!(q["waiting"], rest["waiting"]);
        assert_eq!(q["generation"], rest["generation"]);
        let mut empty = continuation(&q);
        empty["next_job"] = Value::Null;
        assert_eq!(
            call(&client, "queue_continue", empty, true).await["status"],
            400
        );
        let mut missing = continuation(&q);
        missing.as_object_mut().unwrap().remove("removed_job");
        let invalid = client
            .call_tool(
                CallToolRequestParams::new("queue_continue")
                    .with_arguments(missing.as_object().unwrap().clone()),
            )
            .await;
        assert!(invalid.is_err() || invalid.unwrap().is_error == Some(true));
        assert_eq!(
            call(
                &client,
                "queue_get",
                json!({"printer_id":"not-a-printer"}),
                true
            )
            .await["status"],
            404
        );
        // No command is authorized while disconnected or before a fresh report.
        control(&peer, json!({"disconnect":true})).await;
        wait_value(&base, "/api/queue", "/printer/ready_to_print", json!(false)).await;
        assert_eq!(
            call(&client, "queue_continue", continuation(&q), true).await["status"],
            409
        );
        assert_eq!(read(&peer, "").await["count"], 0);
        wait_value(
            &base,
            "/api/queue",
            "/printer/connection",
            json!("synchronizing"),
        )
        .await;
        assert_eq!(
            call(&client, "queue_continue", continuation(&q), true).await["status"],
            409
        );
        control(&peer, json!({})).await;
        wait_value(&base, "/api/queue", "/allowed/next", json!(true)).await;
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        let first = continuation(&q);
        let (a, b) = tokio::join!(
            call(&client, "queue_continue", first.clone(), false),
            call(&client, "queue_continue", first.clone(), false)
        );
        assert_eq!(a["data"]["current"]["id"], q["waiting"][0]["id"]);
        assert_eq!(b["data"]["current"]["id"], q["waiting"][0]["id"]);
        wait_value(&peer, "", "/count", json!(1)).await;
        call(&client, "queue_continue", first.clone(), false).await;
        assert_eq!(read(&peer, "").await["count"], 1);
        control(&peer, json!({"state":"RUNNING"})).await;
        wait_value(&base, "/api/queue", "/current/state", json!("printing")).await;
        control(&peer, json!({"state":"FINISH"})).await;
        wait_value(
            &base,
            "/api/queue",
            "/current/state",
            json!("awaiting_removal"),
        )
        .await;
        assert_eq!(read(&peer, "").await["count"], 1);
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        for key in ["next_job", "removed_job", "epoch"] {
            let mut wrong = continuation(&q);
            wrong[key] = json!("stale-target");
            assert_eq!(
                call(&client, "queue_continue", wrong, true).await["status"],
                409
            );
        }
        let mut stale = continuation(&q);
        stale["generation"] = json!(-1);
        assert_eq!(
            call(&client, "queue_continue", stale, true).await["status"],
            409
        );
        // A held head must not complete the old job or start another material.
        let inventory = read(&base, "/api/printers/p1/ams").await;
        let slot = inventory["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["slot_index"] == 0)
            .unwrap();
        let path = format!(
            "{base}/api/printers/p1/ams/{}",
            slot["id"].as_str().unwrap()
        );
        let http = reqwest::Client::new();
        assert_eq!(
            http.put(&path)
                .json(&json!({"revision":slot["revision"],"filament_id":null}))
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
        let held =
            call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        assert!(!held["waiting"][0]["hold_reason"].is_null());
        assert_eq!(
            call(&client, "queue_continue", continuation(&held), true).await["status"],
            409
        );
        assert_eq!(
            read(&base, "/api/queue").await["current"]["id"],
            q["current"]["id"]
        );
        assert_eq!(read(&peer, "").await["count"], 1);
        let inventory = read(&base, "/api/printers/p1/ams").await;
        let current_slot = inventory["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == slot["id"])
            .unwrap();
        assert_eq!(
            http.put(&path)
                .json(
                    &json!({"revision":current_slot["revision"],"filament_id":slot["filament_id"]})
                )
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        call(&client, "queue_continue", continuation(&q), false).await;
        wait_value(&peer, "", "/count", json!(2)).await;
        assert_eq!(
            read(&peer, "").await["prints"][1]["ams_mapping"],
            json!([0])
        );
        // An old accepted request cannot switch the now-current job back.
        assert_eq!(
            call(&client, "queue_continue", first, true).await["status"],
            409
        );
        control(&peer, json!({"state":"RUNNING"})).await;
        wait_value(&base, "/api/queue", "/current/state", json!("printing")).await;
        control(&peer, json!({"state":"FINISH"})).await;
        wait_value(
            &base,
            "/api/queue",
            "/current/state",
            json!("awaiting_removal"),
        )
        .await;
        let q = call(&client, "queue_get", json!({"printer_id":"p1"}), false).await["data"].clone();
        assert_eq!(q["waiting"], json!([]));
        let finish = continuation(&q);
        assert!(
            call(&client, "queue_continue", finish.clone(), false).await["data"]["current"]
                .is_null()
        );
        assert!(call(&client, "queue_continue", finish, false).await["data"]["current"].is_null());
        assert_eq!(read(&peer, "").await["count"], 2);
        client.cancel().await.unwrap();
    });
    assert_eq!(rig.broker.prints().len(), 2);
    assert_eq!(rig.ftp.uploads().len(), 2);
    assert!(rig.queue()["current"].is_null());
    assert_eq!(rig.queue()["waiting"], json!([]));
}

#[test]
fn support_plate_conditions() {
    let mut rig = common::Rig::new("mcp-support");
    rig.launch();
    rig.seed();
    let id = common::id(&rig.plate).to_owned();
    let interface = common::id(&rig.materials[1]).to_owned();
    let base = rig.base.clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "{base}/mcp"
            )))
            .await
            .unwrap();
        for enabled in [false, true] {
            let mut plate = read(&base, &format!("/api/plates/{id}")).await;
            plate.as_object_mut().unwrap().remove("id");
            plate["conditions"]["support_enabled"] = json!(enabled);
            plate["conditions"]["support_interface_filament_id"] = json!(interface);
            let saved =
                call(&client, "plate_save", json!({"id":id,"plate":plate}), false).await["data"]
                    .clone();
            assert_eq!(saved["conditions"]["support_enabled"], enabled);
            assert_eq!(
                saved["conditions"]["support_interface_filament_id"],
                interface
            );
            assert_eq!(read(&base, &format!("/api/plates/{id}")).await, saved);
            assert_eq!(
                call(&client, "plate_get", json!({"id":id}), false).await["data"],
                saved
            );
        }
        client.cancel().await.unwrap();
    });
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}

#[test]
fn named_role_plate_conditions_round_trip_and_reject_unknown_assignments() {
    let mut rig = common::Rig::new("mcp-roles");
    rig.launch();
    rig.seed();
    rig.files
        .lock()
        .unwrap()
        .insert("role.3mf".into(), common::fixture("material-roles.3mf"));
    let base = rig.base.clone();
    let primary = rig.materials[0]["id"].clone();
    let secondary = rig.materials[1]["id"].clone();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client = ().serve(StreamableHttpClientTransport::from_uri(format!("{base}/mcp"))).await.unwrap();
        let mut plate = call(&client,"plate_save",json!({"plate":{"name":"MCP roles","models":[{"name":"role.3mf","source":"role.3mf","quantity":2}],"conditions":{"filament_id":primary,"secondary_filament_id":secondary}}}),false).await["data"].clone();
        let id = plate["id"].as_str().unwrap().to_owned();
        for secondary in [Value::Null,primary,secondary] {
            let mut edit = common::edit(&plate);
            edit["conditions"]["secondary_filament_id"] = secondary.clone();
            plate = call(&client,"plate_save",json!({"id":id,"plate":edit}),false).await["data"].clone();
            assert_eq!(plate["conditions"]["secondary_filament_id"],secondary);
            assert_eq!(plate["models"][0]["roles"],json!(["primary","secondary"]));
            assert_eq!(call(&client,"plate_get",json!({"id":id}),false).await["data"],plate);
            assert_eq!(read(&base,&format!("/api/plates/{id}")).await,plate);
        }
        let mut bad = common::edit(&plate);
        bad["conditions"]["secondary_filament_id"] = json!("missing");
        call(&client,"plate_save",json!({"id":id,"plate":bad}),true).await;
        assert_eq!(read(&base,&format!("/api/plates/{id}")).await,plate);
        client.cancel().await.unwrap();
    });
    assert!(rig.broker.prints().is_empty() && rig.ftp.uploads().is_empty());
}
