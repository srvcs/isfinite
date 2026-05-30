use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-isfinite";
pub const CONCERN: &str = "validation: is the number finite";
pub const DEPENDS_ON: &[&str] = &["srvcs-isnumber"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub isnumber_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    #[schema(value_type = Object)]
    pub value: Value,
}

#[derive(Serialize, ToSchema)]
pub struct PredicateResponse {
    #[schema(value_type = Object)]
    pub value: Value,
    pub result: bool,
}

fn ok(value: Value, result: bool) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "value": value, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

/// Forward a dependency's response verbatim (used to propagate `422` for invalid
/// input, so isfinite reports the same rejection the leaf dependency did).
fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask the `srvcs-isnumber` dependency with `payload` for its boolean `result`,
/// mapping its failures to the response this service should return.
async fn ask(url: &str, payload: &Value, dependency: &str) -> Result<bool, Response> {
    match client::call(url, payload).await {
        Err(DepError::Unreachable) => Err(degraded(dependency)),
        Ok((200, body)) => Ok(body.get("result").and_then(Value::as_bool).unwrap_or(false)),
        // Invalid input propagates from the leaf dependency; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded(dependency)),
    }
}

/// `POST /` — is `value` finite?
///
/// JSON numbers cannot encode infinities or NaN, so any value `srvcs-isnumber`
/// accepts as a number is finite by construction. This service is therefore
/// near-degenerate: it delegates "is this a number" to `srvcs-isnumber` and
/// reports that verdict as its `result`. If the dependency is unreachable, it
/// reports itself degraded rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = PredicateResponse),
        (status = 422, description = "value is not a valid number (forwarded)"),
        (status = 500, description = "an upstream dependency returned an unexpected response"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let is_number = match ask(
        &deps.isnumber_url,
        &json!({ "value": req.value }),
        "srvcs-isnumber",
    )
    .await
    {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // A JSON number is always finite, so finiteness reduces to "is a number".
    ok(req.value, is_number)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, PredicateResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_concern_and_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-isfinite");
        assert_eq!(info.concern, "validation: is the number finite");
        assert_eq!(info.depends_on, vec!["srvcs-isnumber"]);
    }
}
