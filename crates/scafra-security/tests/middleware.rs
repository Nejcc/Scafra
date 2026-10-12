use axum::{
    body::{to_bytes, Body},
    http::{HeaderValue, Method, Request, StatusCode},
    routing::get,
    Router,
};
use scafra_security::{layer, SecurityConfig};
use tower::ServiceExt;

const BEARER_SECRET: &str = "bearer-sentinel-secret";
const BASIC_PASSWORD_SECRET: &str = "basic-sentinel-secret";

fn protected_router(config: SecurityConfig) -> Router {
    layer(
        Router::new().route("/private", get(|| async { "private response" })),
        config,
    )
}

async fn send(
    router: &Router,
    method: Method,
    path: &str,
    authorization: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).expect("request should build"))
        .await
        .expect("router should respond")
}

async fn response_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    String::from_utf8(body.to_vec()).expect("response body should be UTF-8")
}

#[tokio::test]
async fn bearer_authentication_accepts_only_a_well_formed_matching_token() {
    let config = SecurityConfig {
        enabled: true,
        bearer_token: Some(BEARER_SECRET.to_owned()),
        ..SecurityConfig::default()
    };
    let router = protected_router(config);

    let authorized = send(
        &router,
        Method::GET,
        "/private",
        Some("Bearer bearer-sentinel-secret"),
    )
    .await;
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = response_body(authorized).await;
    assert!(body == "private response");
    assert!(!body.contains(BEARER_SECRET));

    for credential in [
        None,
        Some("Bearer"),
        Some("Bearer wrong-token"),
        Some("Digest bearer-sentinel-secret"),
        Some("Bearer malformed token"),
    ] {
        let response = send(&router, Method::GET, "/private", credential).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response_body(response).await;
        assert!(!body.contains(BEARER_SECRET));
        assert!(!body.contains("wrong-token"));
    }
}

#[tokio::test]
async fn basic_authentication_accepts_only_matching_decoded_credentials() {
    let config = SecurityConfig {
        enabled: true,
        basic: scafra_security::BasicAuthConfig {
            username: Some("user".to_owned()),
            password: Some(BASIC_PASSWORD_SECRET.to_owned()),
        },
        ..SecurityConfig::default()
    };
    let router = protected_router(config);

    let authorized = send(
        &router,
        Method::GET,
        "/private",
        Some("Basic dXNlcjpiYXNpYy1zZW50aW5lbC1zZWNyZXQ="),
    )
    .await;
    assert_eq!(authorized.status(), StatusCode::OK);

    for credential in [
        None,
        Some("Basic !!!"),
        Some("Basic d3Jvbmc6YmFzaWMtc2VudGluZWwtc2VjcmV0"),
        Some("Basic dXNlcjpiYWRwYXNz"),
    ] {
        let response = send(&router, Method::GET, "/private", credential).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response_body(response).await;
        assert!(!body.contains(BASIC_PASSWORD_SECRET));
    }
}

#[tokio::test]
async fn multiple_authorization_values_and_unknown_schemes_fail_closed() {
    let router = protected_router(SecurityConfig {
        enabled: true,
        bearer_token: Some(BEARER_SECRET.to_owned()),
        ..SecurityConfig::default()
    });
    let mut headers = axum::http::HeaderMap::new();
    headers.append(
        "authorization",
        HeaderValue::from_static("Bearer bearer-sentinel-secret"),
    );
    headers.append(
        "authorization",
        HeaderValue::from_static("Bearer another-token"),
    );
    let mut request = Request::builder()
        .uri("/private")
        .body(Body::empty())
        .expect("request should build");
    *request.headers_mut() = headers;
    let response = router
        .oneshot(request)
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response_body(response).await;
    assert!(!body.contains(BEARER_SECRET));

    let unsupported_router = protected_router(SecurityConfig {
        enabled: true,
        bearer_token: Some(BEARER_SECRET.to_owned()),
        ..SecurityConfig::default()
    });
    let unsupported = send(
        &unsupported_router,
        Method::GET,
        "/private",
        Some("Negotiate token"),
    )
    .await;
    assert_eq!(unsupported.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_configuration_still_denies_and_hidden_denials_use_not_found() {
    let invalid = protected_router(SecurityConfig {
        enabled: true,
        ..SecurityConfig::default()
    });
    let response = send(&invalid, Method::GET, "/private", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let hidden = protected_router(SecurityConfig {
        enabled: true,
        hide_unauthorized: true,
        bearer_token: Some(BEARER_SECRET.to_owned()),
        ..SecurityConfig::default()
    });
    let response = send(&hidden, Method::GET, "/private", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_body(response).await;
    assert!(!body.contains(BEARER_SECRET));
}
