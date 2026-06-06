use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Embedded OpenAPI 3.1.0 spec — the canonical schema for api.fastcrw.com.
/// Served at GET /openapi.json. Pulled in at compile time so the binary is
/// self-contained (no runtime filesystem dependency).
///
/// The spec lives INSIDE the crate (`crates/crw-server/openapi/`) so it is
/// included in the published `.crate` tarball — an earlier `../../../../docs/`
/// path reached outside the crate and broke `cargo publish` (the workspace-root
/// docs/ dir isn't packaged). `crates/crw-server/openapi/*` is kept byte-equal
/// to the repo-root `docs/*` copy the docs site serves; a CI guard enforces it.
const OPENAPI_3_1: &str = include_str!("../../openapi/openapi.json");

/// Embedded OpenAPI 3.0.3 downgrade — for tools that don't yet grok 3.1
/// (Postman <11, Insomnia <2024, older openapi-generator). Same paths/schemas,
/// only the version banner and a couple of JSON-Schema-2020 idioms differ.
/// Served at GET /openapi-3.0.json.
const OPENAPI_3_0: &str = include_str!("../../openapi/openapi-3.0.json");

/// GET /openapi.json — serve the 3.1.0 spec.
pub async fn serve_openapi_3_1() -> Response {
    json_response(OPENAPI_3_1)
}

/// GET /openapi-3.0.json — serve the hand-downgraded 3.0.3 spec.
pub async fn serve_openapi_3_0() -> Response {
    json_response(OPENAPI_3_0)
}

fn json_response(body: &'static str) -> Response {
    let mut resp = (StatusCode::OK, body).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    resp
}
