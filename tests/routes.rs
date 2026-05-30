use axum::body::Body;
use axum::extract::Json as ExtractJson;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_isfinite::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

/// Spin up a mock `srvcs-isnumber` that actually COMPUTES its verdict from the
/// request body: it reports `result: true` iff `value` is a JSON number. This
/// genuinely exercises isfinite's delegation rather than parroting a fixed
/// answer. Returns the mock's base URL.
async fn spawn_isnumber() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|ExtractJson(req): ExtractJson<Value>| async move {
            let is_number = req.get("value").map(Value::is_number).unwrap_or(false);
            (StatusCode::OK, Json(json!({ "result": is_number })))
        }),
    );
    spawn(app).await
}

/// Spin up a mock that answers `POST /` with a fixed status + body.
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    spawn(app).await
}

async fn spawn(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(isnumber_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            isnumber_url: isnumber_url.to_string(),
        },
    )
}

async fn eval(isnumber_url: &str, value: Value) -> (StatusCode, Value) {
    let res = app(isnumber_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": value }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

// A base URL with nothing listening — exercises the degraded path.
const DEAD_URL: &str = "http://127.0.0.1:1";

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn index_ok() {
    assert_eq!(status_of("/").await, StatusCode::OK);
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn metrics_ok() {
    assert_eq!(status_of("/metrics").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

#[tokio::test]
async fn generates_request_id_when_absent() {
    let res = app(DEAD_URL)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        res.headers().contains_key("x-request-id"),
        "response must carry a generated x-request-id"
    );
}

// Spec: isnumber {"result":true} -> result true.
#[tokio::test]
async fn finite_when_isnumber_agrees() {
    let isnumber = spawn_isnumber().await;
    for v in [json!(0), json!(4), json!(-7), json!(2.5), json!(1e9)] {
        let (status, body) = eval(&isnumber, v.clone()).await;
        assert_eq!(status, StatusCode::OK, "value {v} should be OK");
        assert_eq!(body["result"], true, "value {v} should be finite");
        assert_eq!(body["value"], v, "value {v} echoed back");
    }
}

// Spec: isnumber {"result":false} -> result false.
#[tokio::test]
async fn not_finite_when_isnumber_says_not_a_number() {
    let isnumber = spawn_isnumber().await;
    for v in [json!("nope"), json!(true), json!(null), json!([1, 2])] {
        let (status, body) = eval(&isnumber, v.clone()).await;
        assert_eq!(status, StatusCode::OK, "value {v} should be OK");
        assert_eq!(body["result"], false, "value {v} is not a number");
    }
}

// 422 forwarded when isnumber rejects the input as unprocessable.
#[tokio::test]
async fn forwards_422_from_isnumber() {
    let isnumber = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, body) = eval(&isnumber, json!("nope")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "value is not a number");
}

// Spec: dead url -> 503.
#[tokio::test]
async fn degrades_when_isnumber_is_unreachable() {
    let (status, body) = eval(DEAD_URL, json!(4)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-isnumber");
}

// An unexpected upstream status degrades rather than guessing.
#[tokio::test]
async fn degrades_on_unexpected_upstream_status() {
    let isnumber = spawn_fixed(StatusCode::INTERNAL_SERVER_ERROR, json!({})).await;
    let (status, _) = eval(&isnumber, json!(4)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
