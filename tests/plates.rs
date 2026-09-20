use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use orca_server::plates::Store;
use serde_json::Value;
use tower::ServiceExt;

fn upload(method: &str, path: &str, name: &str, data: &str) -> Request<Body> {
    let body = format!(
        "--orca\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n--orca\r\nContent-Disposition: form-data; name=\"models\"; filename=\"parts/box.stl\"\r\nContent-Type: model/stl\r\n\r\n{data}\r\n--orca--\r\n"
    );
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "multipart/form-data; boundary=orca")
        .body(Body::from(body))
        .unwrap()
}

async fn json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn multipart_save_search_download_and_restart_use_the_same_store() {
    let root = tempfile::tempdir().unwrap();
    let app = orca_server::app_with_store(Store::open(root.path()).unwrap());
    let response = app
        .clone()
        .oneshot(upload(
            "POST",
            "/api/plates",
            "Desk box",
            include_str!("fixtures/triangle.stl"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let saved = json(response).await;
    let id = saved["id"].as_str().unwrap();
    let file = saved["models"][0]["id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/plates/{id}/files/{file}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        include_bytes!("fixtures/triangle.stl").as_slice()
    );
    let app = orca_server::app_with_store(Store::open(root.path()).unwrap());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/plates?q=dsbx")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await[0]["id"], saved["id"]);
    let replacement = serde_json::json!({"name":"Renamed","version":saved["version"],"models":[{
        "id":saved["models"][0]["id"],"name":"parts/box.stl","source":null,"quantity":2}]});
    let replace = |value: &Value| {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/plates/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(replace(&replacement)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let changed = json(response).await;
    assert_eq!(changed["name"], "Renamed");
    assert_ne!(changed["version"], saved["version"]);
    assert_eq!(changed["models"][0]["quantity"], 2);
    assert_eq!(
        app.clone()
            .oneshot(replace(&replacement))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/plates/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await, changed);
}

#[tokio::test]
async fn bad_upload_unknown_plate_and_unlisted_files_fail_without_losing_data() {
    let root = tempfile::tempdir().unwrap();
    let app = orca_server::app_with_store(Store::open(root.path()).unwrap());
    let response = app
        .clone()
        .oneshot(upload(
            "POST",
            "/api/plates",
            "Valid",
            include_str!("fixtures/triangle.stl"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let saved = json(response).await;
    let id = saved["id"].as_str().unwrap();
    let response=app.clone().oneshot(Request::builder().method("PUT").uri(format!("/api/plates/{id}"))
        .header("content-type","application/json").body(Body::from(serde_json::json!({"name":"Invalid","version":saved["version"],"models":[{"id":saved["models"][0]["id"],"name":"parts/box.stl","source":null,"quantity":0}]}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(json(response).await["error"].is_string());
    for path in [
        format!("/api/plates/{id}/files/plate.json"),
        format!("/api/plates/{}", uuid::Uuid::new_v4()),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/plates/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json(response).await, saved);
}

#[tokio::test]
async fn browser_writes_must_come_from_the_serving_origin() {
    let root = tempfile::tempdir().unwrap();
    let app = orca_server::app_with_store(Store::open(root.path()).unwrap());
    for origin in ["https://unrelated.example", "null", "not a URI"] {
        let mut request = upload(
            "POST",
            "/api/plates",
            "Injected",
            include_str!("fixtures/triangle.stl"),
        );
        request
            .headers_mut()
            .insert("host", "orca.local:3000".parse().unwrap());
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert!(
        Store::open(root.path())
            .unwrap()
            .list("")
            .unwrap()
            .is_empty()
    );
    let mut request = upload(
        "POST",
        "/api/plates",
        "Allowed",
        include_str!("fixtures/triangle.stl"),
    );
    request
        .headers_mut()
        .insert("host", "orca.local:3000".parse().unwrap());
    request
        .headers_mut()
        .insert("origin", "http://orca.local:3000".parse().unwrap());
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );
}
