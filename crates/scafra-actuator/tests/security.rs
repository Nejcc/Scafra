use axum::{
    body::{to_bytes, Body},
    http::{HeaderValue, Request, StatusCode},
};
use scafra_actuator::{
    router_with_health_registry, ActuatorConfig, ActuatorSecurity, HealthConfig, HealthRegistry,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

const TOKEN: &str = "actuator-auth-test-token";
const ACTUATOR_PATHS: [&str; 12] = [
    "/health",
    "/actuator/health",
    "/live",
    "/health/live",
    "/actuator/health/liveness",
    "/ready",
    "/health/ready",
    "/actuator/health/readiness",
    "/info",
    "/actuator/info",
    "/metrics",
    "/actuator/metrics",
];

fn protected_router(token: Option<&str>) -> (axum::Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_check = Arc::clone(&calls);
    let mut registry = HealthRegistry::new();
    registry
        .register("dependency", move || {
            let calls = Arc::clone(&calls_for_check);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            }
        })
        .expect("the test registry should accept its check");
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["dependency".to_owned()],
            ..HealthConfig::default()
        },
        security: ActuatorSecurity {
            enabled: true,
            bearer_token: token.map(str::to_owned),
        },
        ..ActuatorConfig::all()
    };
    (
        router_with_health_registry(&config, registry)
            .expect("the graph-scoped test router should build"),
        calls,
    )
}

async fn request(
    router: &axum::Router,
    path: &str,
    authorization: Option<HeaderValue>,
) -> (StatusCode, String) {
    let mut request = Request::builder().uri(path);
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).expect("request should build"))
        .await
        .expect("router should respond");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    (
        status,
        String::from_utf8(body.to_vec()).expect("response should be UTF-8"),
    )
}

fn bearer(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("authorization value should be a valid header value")
}

#[tokio::test]
async fn disabled_security_allows_every_installed_actuator_route() {
    let mut registry = HealthRegistry::new();
    registry
        .register("healthy", || async { true })
        .expect("the test registry should accept its check");
    let router = router_with_health_registry(&ActuatorConfig::all(), registry)
        .expect("the graph-scoped test router should build");

    for path in ACTUATOR_PATHS {
        assert_eq!(
            request(&router, path, None).await.0,
            StatusCode::OK,
            "{path}"
        );
    }
}

#[tokio::test]
async fn protected_routes_reject_missing_malformed_and_incorrect_credentials_before_checks() {
    let (router, calls) = protected_router(Some(TOKEN));
    let malformed = HeaderValue::from_bytes(b"\xff")
        .expect("header values may contain opaque bytes that fail text parsing");

    for path in ACTUATOR_PATHS {
        for authorization in [
            None,
            Some(bearer("Basic credentials")),
            Some(bearer("Bearer")),
            Some(malformed.clone()),
            Some(bearer("Bearer wrong-token")),
        ] {
            let (status, body) = request(&router, path, authorization).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
            assert!(body.is_empty(), "unauthorized response should be empty");
            assert!(
                !body.contains(TOKEN),
                "unauthorized response exposed a token"
            );
        }
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn valid_bearer_token_authorizes_every_alias_and_keeps_tokens_out_of_responses() {
    let (router, calls) = protected_router(Some(TOKEN));
    for path in ACTUATOR_PATHS {
        let (status, body) = request(&router, path, Some(bearer(&format!("Bearer {TOKEN}")))).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(!body.contains(TOKEN), "actuator response exposed a token");
    }
    // Aggregate health (2 aliases) and readiness (3 aliases) evaluate checks.
    assert_eq!(calls.load(Ordering::Relaxed), 5);
}

#[tokio::test]
async fn blank_configured_tokens_fail_closed_even_for_direct_router_construction() {
    for configured_token in ["", " \t "] {
        let (router, calls) = protected_router(Some(configured_token));
        for candidate in [None, Some(bearer("Bearer")), Some(bearer("Bearer  "))] {
            let (status, body) = request(&router, "/ready", candidate).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body.is_empty());
        }
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn enabled_security_without_a_configured_token_fails_closed() {
    let (router, calls) = protected_router(None);
    let (status, body) =
        request(&router, "/health", Some(bearer(&format!("Bearer {TOKEN}")))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
